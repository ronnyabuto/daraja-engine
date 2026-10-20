use super::*;
use crate::{amount::Amount, valuation::PaymentQuote};

impl Repository {
    /// An existing identity can only replay the identical frozen quote.
    pub async fn save_payment_quote(&self, quote: &PaymentQuote, now: Timestamp) -> Result<()> {
        let mut tx = self.write().await?;
        if let Some(row) = sqlx::query("SELECT data FROM payment_quotes WHERE id=?")
            .bind(quote.id().as_str())
            .fetch_optional(&mut *tx)
            .await?
        {
            if decode::<PaymentQuote>(row.try_get("data")?)? != *quote {
                return Err(Error::IdempotencyConflict.into());
            }
            return Ok(());
        }
        quote.validate_dispatch(now)?;
        sqlx::query("INSERT INTO payment_quotes(id,data) VALUES(?,?)")
            .bind(quote.id().as_str())
            .bind(encode(quote)?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn payment_quote(&self, id: &Id) -> Result<PaymentQuote> {
        let row = sqlx::query("SELECT data FROM payment_quotes WHERE id=?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::NotFound)?;
        decode(row.try_get("data")?)
    }
    /// Trusted fee evidence boundary. A callback alone must never call this API.
    /// Unknown fees conservatively consume the reserve. An immutable record preserves both values.
    pub async fn record_payout_fee(
        &self,
        id: &Id,
        actual: Option<Amount>,
        now: Timestamp,
    ) -> Result<Amount> {
        let mut tx = self.write().await?;
        let row = sqlx::query("SELECT data FROM outbound_payouts WHERE id=?")
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
        let payout: Payout = decode(row.try_get("data")?)?;
        if payout.state() != OutboundState::Paid {
            return Err(Error::InvalidTransition.into());
        }
        let q = payout.dispatch().quote().ok_or(Error::NotFound)?;
        let spent = q.total_spent(actual);
        let total = match &spent {
            Ok(value) => Some(encode(value)?),
            Err(Error::SettlementHeld) => None,
            Err(error) => return Err((*error).into()),
        };
        let fee = actual.map(|a| encode(&a)).transpose()?;
        if let Some(row) =
            sqlx::query("SELECT actual_fee,total_spent FROM payout_fee_audit WHERE payout_id=?")
                .bind(id.as_str())
                .fetch_optional(&mut *tx)
                .await?
        {
            if row.try_get::<Option<String>, _>("actual_fee")? != fee
                || row.try_get::<Option<String>, _>("total_spent")? != total
            {
                return Err(Error::IdempotencyConflict.into());
            }
            return spent.map_err(Into::into);
        }
        sqlx::query("INSERT INTO payout_fee_audit(payout_id,actual_fee,total_spent,recorded_at) VALUES(?,?,?,?)").bind(id.as_str()).bind(fee).bind(total).bind(time(now)?).execute(&mut *tx).await?;
        tx.commit().await?;
        spent.map_err(Into::into)
    }
}

//! Trusted operator boundaries; no method accepts customer receipt-only claims.
use super::*;
use crate::{
    amount::{Amount, TariffSnapshot, Unit},
    identity::ReceiptId,
    inbound::InboundState,
};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DepositPolicy {
    pub kes_grace_seconds: u64,
    /// Zero disables this risk policy. Amount is KES minor units.
    pub high_value_minor: u64,
    pub hold_seconds: u64,
    pub velocity_window_seconds: u64,
    /// High-value unverified deposits above this count require audited release.
    pub velocity_limit: u64,
}

/// Only an authenticated operator may supply reviewed ownership and destination
/// evidence covering EVERY reserved receipt. A receipt number proves neither.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RefundAuthorization {
    pub operator: Id,
    pub owner: Id,
    pub evidence: Id,
    pub destination: Id,
    pub receipts: Vec<ReceiptId>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RefundRequest {
    pub id: Id,
    pub authorization: RefundAuthorization,
    pub tariff: TariffSnapshot,
    pub gross: Amount,
    pub net: Amount,
}

impl Repository {
    /// Trusted deployment/operator configuration, persisted across all handles/restarts.
    pub async fn set_deposit_policy(
        &self,
        policy: &DepositPolicy,
        operator: &Id,
        now: Timestamp,
    ) -> Result<()> {
        if policy.high_value_minor != 0
            && (policy.hold_seconds == 0
                || policy.velocity_window_seconds == 0
                || policy.velocity_limit == 0)
        {
            return Err(Error::InvalidAmount.into());
        }
        let mut tx = self.write().await?;
        sqlx::query("INSERT INTO deposit_policy VALUES(1,?) ON CONFLICT(singleton) DO UPDATE SET data=excluded.data")
            .bind(encode(policy)?).execute(&mut *tx).await?;
        audit(
            &mut tx,
            operator,
            &format!("DEPOSIT_POLICY_UPDATED:{}", encode(policy)?),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Reserves whole verified exception receipts. Quote allocations are reserved
    /// together, disabling further top-up allocation and minting on that quote.
    pub async fn request_refund(
        &self,
        id: &Id,
        mut authorization: RefundAuthorization,
        tariff: TariffSnapshot,
        now: Timestamp,
    ) -> Result<RefundRequest> {
        authorization.receipts.sort();
        if authorization.receipts.is_empty()
            || authorization.receipts.windows(2).any(|r| r[0] == r[1])
        {
            return Err(Error::InvalidAmount.into());
        }
        tariff.fee.require_kes()?;
        let mut tx = self.write().await?;
        let previous: Option<String> =
            sqlx::query_scalar("SELECT data FROM refund_requests WHERE id=?")
                .bind(id.as_str())
                .fetch_optional(&mut *tx)
                .await?;
        let mut gross = Amount::new(0, Unit::KesMinor);
        let mut quote = None;
        for receipt in &authorization.receipts {
            let data: String =
                sqlx::query_scalar("SELECT data FROM mpesa_receipts_ledger WHERE receipt=?")
                    .bind(receipt.as_str())
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or(Error::NotFound)?;
            let record: ConfirmedReceipt = decode(&data)?;
            if record.state == DepositState::Settled || now < record.deposit.evidence().verified_at
            {
                return Err(Error::InvalidTransition.into());
            }
            gross = gross.checked_add(record.deposit.evidence().amount)?;
            if let Allocation::Quote(q) = &record.allocation {
                if quote.as_ref().is_some_and(|old| old != q) {
                    return Err(Error::EvidenceMismatch.into());
                }
                quote = Some(q.clone());
            }
        }
        let net = gross.checked_sub(tariff.fee)?;
        if net.value() == 0 {
            return Err(Error::InvalidAmount.into());
        }
        net.whole_kes()?;
        let request = RefundRequest {
            id: id.clone(),
            authorization,
            tariff,
            gross,
            net,
        };
        let data = encode(&request)?;
        if let Some(previous) = previous {
            if previous != data {
                return Err(Error::IdempotencyConflict.into());
            }
            return Ok(request);
        }
        if let Some(q) = &quote {
            let mut a = load_accounting(&mut tx, Some(q)).await?;
            let actual: Vec<_> = a.receipts.keys().cloned().collect();
            if actual != request.authorization.receipts {
                return Err(Error::EvidenceMismatch.into());
            }
            a.reserve_refund(q)?;
            let version: i64 = sqlx::query_scalar("SELECT version FROM inbound_quotes WHERE id=?")
                .bind(q.as_str())
                .fetch_one(&mut *tx)
                .await?;
            persist_accounting(&mut tx, &a, q, version).await?;
        }
        sqlx::query(
            "INSERT INTO refund_requests(id,data,state,created_at) VALUES(?,?,'EXECUTION_GATED',?)",
        )
        .bind(id.as_str())
        .bind(data)
        .bind(time(now)?)
        .execute(&mut *tx)
        .await?;
        for receipt in &request.authorization.receipts {
            sqlx::query("INSERT INTO refund_reservations(receipt,refund_id) VALUES(?,?)")
                .bind(receipt.as_str())
                .bind(id.as_str())
                .execute(&mut *tx)
                .await?;
        }
        audit(&mut tx, id, "REFUND_RESERVED_EXECUTION_GATED", now).await?;
        tx.commit().await?;
        Ok(request)
    }

    pub async fn refund_status(&self, id: &Id) -> Result<String> {
        sqlx::query_scalar("SELECT state FROM refund_requests WHERE id=?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::NotFound.into())
    }

    /// Authenticated operator review releases velocity/reversal blocks, never the
    /// independent verification, exact-value, FX or time-based hold predicates.
    pub async fn release_deposit_hold(
        &self,
        quote: &Id,
        operator: &Id,
        evidence: &Id,
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.write().await?;
        changed(
            sqlx::query("DELETE FROM deposit_blocks WHERE quote_id=?")
                .bind(quote.as_str())
                .execute(&mut *tx)
                .await?
                .rows_affected(),
        )?;
        audit(
            &mut tx,
            quote,
            &format!("HOLD_RELEASED:{}:{}", operator.as_str(), evidence.as_str()),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// A reversal observation raises an incident; minted bearer value is never
    /// clawed back and no timer/reversal callback declares a refund successful.
    pub async fn report_reversal(
        &self,
        receipt: &ReceiptId,
        evidence: &Id,
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.write().await?;
        let data: String =
            sqlx::query_scalar("SELECT data FROM mpesa_receipts_ledger WHERE receipt=?")
                .bind(receipt.as_str())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error::NotFound)?;
        let r: ConfirmedReceipt = decode(&data)?;
        sqlx::query("INSERT INTO deposit_alerts(receipt,kind,evidence,created_at) VALUES(?,'REVERSAL_REPORTED',?,?) ON CONFLICT DO NOTHING").bind(receipt.as_str()).bind(evidence.as_str()).bind(time(now)?).execute(&mut *tx).await?;
        if let Some(q) = &r.deposit.evidence().quote_id {
            sqlx::query(
                "INSERT INTO deposit_blocks VALUES(?,'REVERSAL_REPORTED',?) ON CONFLICT DO NOTHING",
            )
            .bind(q.as_str())
            .bind(time(now)?)
            .execute(&mut *tx)
            .await?;
        }
        audit(&mut tx, &Id::new(receipt.as_str())?, "REVERSAL_ALERT", now).await?;
        tx.commit().await?;
        Ok(())
    }
}

pub(super) async fn policy(conn: &mut SqliteConnection) -> Result<DepositPolicy> {
    let data: Option<String> =
        sqlx::query_scalar("SELECT data FROM deposit_policy WHERE singleton=1")
            .fetch_optional(conn)
            .await?;
    Ok(data
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default())
}

pub(super) async fn apply_risk(
    conn: &mut SqliteConnection,
    a: &mut Accounting,
    deposit: &VerifiedDeposit,
    policy: &DepositPolicy,
) -> Result<()> {
    let e = deposit.evidence();
    if policy.high_value_minor == 0
        || e.amount.value() < policy.high_value_minor
        || e.payer.as_ref().is_some_and(|p| p.identity_verified)
    {
        return Ok(());
    }
    // Missing independently verified payer identity shares a conservative bucket.
    let subject = e
        .payer
        .as_ref()
        .map(|p| p.subject.as_str())
        .unwrap_or("UNKNOWN");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM deposit_risk WHERE subject=? AND verified_at>=?")
            .bind(subject)
            .bind(time(Timestamp(
                e.verified_at
                    .0
                    .saturating_sub(policy.velocity_window_seconds),
            ))?)
            .fetch_one(&mut *conn)
            .await?;
    sqlx::query("INSERT INTO deposit_risk VALUES(?,?,?)")
        .bind(e.receipt.as_str())
        .bind(subject)
        .bind(time(e.verified_at)?)
        .execute(&mut *conn)
        .await?;
    if let Some(q) = &e.quote_id {
        if a.quote(q)
            .is_some_and(|q| matches!(q.state(), InboundState::Paid | InboundState::RefundHeld))
        {
            return Ok(());
        }
        a.hold_until(
            q,
            Timestamp(
                e.verified_at
                    .0
                    .checked_add(policy.hold_seconds)
                    .ok_or(Error::Overflow)?,
            ),
        )?;
        if u64::try_from(count).map_err(|_| Error::Overflow)? >= policy.velocity_limit {
            sqlx::query("INSERT INTO deposit_blocks VALUES(?,'VELOCITY',?) ON CONFLICT DO NOTHING")
                .bind(q.as_str())
                .bind(time(e.verified_at)?)
                .execute(&mut *conn)
                .await?;
            sqlx::query("INSERT INTO deposit_alerts(receipt,kind,evidence,created_at) VALUES(?,'VELOCITY','POLICY',?)").bind(e.receipt.as_str()).bind(time(e.verified_at)?).execute(&mut *conn).await?;
            a.blocked = true;
        }
    }
    Ok(())
}

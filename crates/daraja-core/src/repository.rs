//! SQLite transaction boundary. No transport or user callbacks run under a write lock.
use std::{path::Path, time::Duration};

use crate::storage::Stored;
use serde::Serialize;
use sqlx::{
    ConnectOptions, Row, Sqlite, SqliteConnection, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::{
    Error, Timestamp,
    identity::{Id, RequestKey, RequestPayload},
    inbound::{
        Accounting, Allocation, ConfirmedReceipt, DepositState, InboundQuote, ReceiptObservation,
        SettlementEvent, StkAttempt, VerifiedDeposit,
    },
    outbound::{Ambiguity, OutboundState, Payout, PayoutAttestation},
};

#[derive(Debug)]
pub enum RepositoryError {
    Domain(Error),
    Storage,
    Encoding,
    Conflict,
    LeaseLost,
}
impl std::fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RepositoryError {}
impl From<Error> for RepositoryError {
    fn from(e: Error) -> Self {
        Self::Domain(e)
    }
}
impl From<sqlx::Error> for RepositoryError {
    fn from(_: sqlx::Error) -> Self {
        Self::Storage
    }
}
impl From<serde_json::Error> for RepositoryError {
    fn from(_: serde_json::Error) -> Self {
        Self::Encoding
    }
}
type Result<T> = std::result::Result<T, RepositoryError>;
fn encode(value: &impl Serialize) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
fn decode<T: Stored>(value: &str) -> Result<T> {
    Ok(crate::storage::Value::<T>::deserialize_json(value)?)
}
fn time(value: Timestamp) -> Result<i64> {
    i64::try_from(value.0).map_err(|_| Error::Overflow.into())
}
fn state(value: &impl Serialize) -> Result<String> {
    Ok(encode(value)?.trim_matches('"').to_owned())
}
fn changed(rows: u64) -> Result<()> {
    if rows == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

#[derive(Clone)]
pub struct Repository {
    pool: SqlitePool,
}

pub enum StkChange {
    Start(StkAttempt),
    Acknowledge {
        attempt: Id,
        merchant: Id,
        checkout: Id,
    },
    Unknown(Id),
    Finish {
        attempt: Id,
        succeeded: bool,
    },
}
pub enum PayoutChange {
    Ambiguous(Ambiguity),
    Acknowledge { originator: Id, conversation: Id },
    DuplicateObserved,
    Escalate,
    Resolve(PayoutAttestation),
}

/// The generation fences workers whose bounded lease expired. Claims grant work,
/// never permission to redispatch an ambiguous payout.
#[derive(Clone, Debug)]
pub struct JobClaim {
    pub id: Id,
    pub kind: String,
    pub subject: Id,
    pub generation: i64,
    pub lease_until: Timestamp,
}

impl Repository {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        if path.as_ref() == Path::new(":memory:") {
            return Err(RepositoryError::Storage);
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .disable_statement_logging();
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|_| RepositoryError::Storage)?;
        Ok(Self { pool })
    }
    pub async fn close(&self) {
        self.pool.close().await;
    }
    async fn write(&self) -> Result<Transaction<'static, Sqlite>> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }

    pub async fn insert_quote(&self, quote: &InboundQuote, now: Timestamp) -> Result<()> {
        // Serialized snapshots are private storage; admit only freshly validated quotes.
        let fresh = InboundQuote::new(
            quote.id().clone(),
            quote.account_ref().clone(),
            quote.shortcode().clone(),
            quote.terms().clone(),
        )?;
        if *quote != fresh {
            return Err(Error::InvalidTransition.into());
        }
        let mut tx = self.write().await?;
        sqlx::query("INSERT INTO inbound_quotes(id,account_ref,state,data) VALUES(?,?,?,?)")
            .bind(quote.id().as_str())
            .bind(quote.account_ref().as_str())
            .bind(state(&quote.state())?)
            .bind(encode(quote)?)
            .execute(&mut *tx)
            .await?;
        audit(&mut tx, quote.id(), "QUOTE_CREATED", now).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn quote(&self, id: &Id) -> Result<(InboundQuote, i64)> {
        let row = sqlx::query("SELECT data,version FROM inbound_quotes WHERE id=?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::NotFound)?;
        Ok((decode(row.try_get("data")?)?, row.try_get("version")?))
    }
    pub async fn payout(&self, id: &Id) -> Result<(Payout, i64)> {
        let row = sqlx::query("SELECT data,version FROM outbound_payouts WHERE id=?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::NotFound)?;
        Ok((decode(row.try_get("data")?)?, row.try_get("version")?))
    }
    pub async fn receipt(
        &self,
        id: &crate::identity::ReceiptId,
    ) -> Result<Option<ConfirmedReceipt>> {
        let data: Option<String> =
            sqlx::query_scalar("SELECT data FROM mpesa_receipts_ledger WHERE receipt=?")
                .bind(id.as_str())
                .fetch_optional(&self.pool)
                .await?;
        data.map(|s| decode(&s)).transpose()
    }
    pub async fn observation(&self, id: &Id) -> Result<ReceiptObservation> {
        let data: String = sqlx::query_scalar("SELECT data FROM receipt_observations WHERE id=?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::NotFound)?;
        decode(&data)
    }
    pub async fn attempt(&self, id: &Id) -> Result<(Id, StkAttempt)> {
        let row = sqlx::query("SELECT quote_id,data FROM stk_attempts WHERE id=?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::NotFound)?;
        Ok((
            Id::new(row.try_get::<String, _>("quote_id")?)?,
            decode(row.try_get("data")?)?,
        ))
    }
    pub async fn events_after(
        &self,
        sequence: i64,
        limit: u32,
    ) -> Result<Vec<(i64, SettlementEvent)>> {
        let rows = sqlx::query("SELECT sequence,data FROM settlement_events WHERE sequence>? ORDER BY sequence LIMIT ?").bind(sequence).bind(limit.min(1000)).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|r| Ok((r.try_get("sequence")?, decode(r.try_get("data")?)?)))
            .collect()
    }
    pub async fn change_stk(
        &self,
        id: &Id,
        version: i64,
        change: StkChange,
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.write().await?;
        let mut accounting = load_accounting(&mut tx, Some(id)).await?;
        let q = accounting.quotes.get_mut(id).ok_or(Error::NotFound)?;
        match change {
            StkChange::Start(a) => q.start_stk(a)?,
            StkChange::Acknowledge {
                attempt,
                merchant,
                checkout,
            } => q.acknowledge_stk(&attempt, merchant, checkout)?,
            StkChange::Unknown(a) => q.stk_unknown(&a)?,
            StkChange::Finish { attempt, succeeded } => q.finish_stk(&attempt, succeeded)?,
        }
        save_quote(&mut tx, q, version).await?;
        for a in q.attempts() {
            sqlx::query("INSERT INTO stk_attempts(id,quote_id,caller_request,merchant_request,checkout_request,next_poll_at,resend_after,data) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET merchant_request=excluded.merchant_request,checkout_request=excluded.checkout_request,data=excluded.data")
                .bind(a.id().as_str()).bind(id.as_str()).bind(a.caller_request().as_str()).bind(a.merchant_request().map(Id::as_str)).bind(a.checkout_request().map(Id::as_str)).bind(time(a.next_poll_at())?).bind(time(a.resend_after())?).bind(encode(a)?).execute(&mut *tx).await?;
            enqueue(
                &mut tx,
                &format!("stk:{}", a.id().as_str()),
                "POLL_STK",
                a.id(),
                a.next_poll_at(),
            )
            .await?;
        }
        audit(&mut tx, id, "STK_CHANGED", now).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn observe(&self, observation: &ReceiptObservation) -> Result<()> {
        let mut tx = self.write().await?;
        let data = encode(observation)?;
        let old: Option<String> =
            sqlx::query_scalar("SELECT data FROM receipt_observations WHERE id=?")
                .bind(observation.observation_id.as_str())
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(old) = old {
            if old != data {
                return Err(Error::IdempotencyConflict.into());
            }
        } else {
            sqlx::query(
                "INSERT INTO receipt_observations(id,receipt,observed_at,data) VALUES(?,?,?,?)",
            )
            .bind(observation.observation_id.as_str())
            .bind(observation.claimed_receipt.as_str())
            .bind(time(observation.observed_at)?)
            .bind(data)
            .execute(&mut *tx)
            .await?;
            enqueue(
                &mut tx,
                &format!("verify:{}", observation.observation_id.as_str()),
                "VERIFY_RECEIPT",
                &observation.observation_id,
                observation.observed_at,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    /// Trusted verifier boundary: callback DTOs cannot authorize this call.
    /// Confirmation and optional settlement are one transaction; held funds can
    /// be confirmed separately and settled later after their predicates pass.
    pub async fn confirm(
        &self,
        deposit: VerifiedDeposit,
        settle_at: Option<Timestamp>,
    ) -> Result<DepositState> {
        let mut tx = self.write().await?;
        let evidence = deposit.evidence();
        let quote_id = evidence.quote_id.clone();
        let receipt = evidence.receipt.clone();
        let now = evidence.verified_at;
        let mut a = load_accounting(&mut tx, quote_id.as_ref()).await?;
        // Global uniqueness is checked even when this receipt belongs to another quote.
        let exists: i64 =
            sqlx::query_scalar("SELECT count(*) FROM mpesa_receipts_ledger WHERE receipt=?")
                .bind(receipt.as_str())
                .fetch_one(&mut *tx)
                .await?;
        if exists != 0 {
            return Err(Error::DuplicateReceipt.into());
        }
        a.confirm(deposit)?;
        let r = a.receipt(&receipt).ok_or(Error::NotFound)?;
        let allocation = match &r.allocation {
            Allocation::Quote(q) => Some(q.as_str()),
            Allocation::Exception => None,
        };
        sqlx::query(
            "INSERT INTO mpesa_receipts_ledger(receipt,quote_id,state,data) VALUES(?,?,?,?)",
        )
        .bind(receipt.as_str())
        .bind(allocation)
        .bind(state(&r.state)?)
        .bind(encode(r)?)
        .execute(&mut *tx)
        .await?;
        if r.state != DepositState::Verified {
            sqlx::query("INSERT INTO unreconciled_deposits(receipt,reason) VALUES(?,?)")
                .bind(receipt.as_str())
                .bind(state(&r.state)?)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(id) = &quote_id {
            let version: i64 = sqlx::query_scalar("SELECT version FROM inbound_quotes WHERE id=?")
                .bind(id.as_str())
                .fetch_one(&mut *tx)
                .await?;
            if let Some(settle_at) = settle_at {
                a.settle(id, settle_at)?;
            }
            persist_accounting(&mut tx, &a, id, version).await?;
        } else if settle_at.is_some() {
            return Err(Error::NotFound.into());
        }
        audit(
            &mut tx,
            &Id::new(receipt.as_str())?,
            "RECEIPT_CONFIRMED",
            now,
        )
        .await?;
        let outcome = a.receipt(&receipt).ok_or(Error::NotFound)?.state;
        tx.commit().await?;
        Ok(outcome)
    }
    pub async fn settle(&self, id: &Id, version: i64, now: Timestamp) -> Result<SettlementEvent> {
        let mut tx = self.write().await?;
        let mut a = load_accounting(&mut tx, Some(id)).await?;
        let event = a.settle(id, now)?;
        persist_accounting(&mut tx, &a, id, version).await?;
        audit(&mut tx, id, "QUOTE_SETTLED", now).await?;
        tx.commit().await?;
        Ok(event)
    }
    /// Returns the original durable operation for an identical request replay.
    /// Successful return is the commit-before-I/O crash boundary. Recovery only
    /// schedules reconciliation, never automatic resubmission.
    pub async fn prepare_payout(&self, key: &RequestKey, payout: &Payout) -> Result<Id> {
        let d = payout.dispatch();
        if key.operation != d.payload().operation {
            return Err(Error::EvidenceMismatch.into());
        }
        let mut tx = self.write().await?;
        if let Some(row) = sqlx::query("SELECT payload_hash,operation_id FROM request_bindings WHERE caller=? AND operation=? AND request_key=?")
            .bind(key.caller.as_str()).bind(key.operation.as_str()).bind(key.key.as_str()).fetch_optional(&mut *tx).await? {
            if row.try_get::<Vec<u8>,_>("payload_hash")? != d.hash().as_bytes() { return Err(Error::IdempotencyConflict.into()); }
            return Ok(Id::new(row.try_get::<String,_>("operation_id")?)?);
        }
        // Revalidate deserialized values before admitting a new dispatch.
        let validated = crate::outbound::DispatchRecord::new(
            d.operation_id().clone(),
            d.payload().clone(),
            d.wire_body().to_vec(),
            d.tariff().clone(),
            d.reserved(),
        )?;
        let fresh = Payout::dispatching(
            validated,
            payout.correlation().caller_request.clone(),
            payout.started_at(),
            payout.next_check_at(),
            payout.manual_after(),
        )?;
        if *payout != fresh {
            return Err(Error::InvalidTransition.into());
        }
        sqlx::query("INSERT INTO outbound_payouts(id,caller_request,state,payload_hash,dispatch,wire_body,next_check_at,manual_after,data) VALUES(?,?,?,?,?,?,?,?,?)")
            .bind(d.operation_id().as_str()).bind(payout.correlation().caller_request.as_str()).bind(state(&payout.state())?).bind(d.hash().as_bytes().as_slice()).bind(encode(d)?).bind(d.wire_body()).bind(time(payout.next_check_at())?).bind(time(payout.manual_after())?).bind(encode(payout)?).execute(&mut *tx).await?;
        bind_request(&mut tx, key, d.payload(), d.operation_id()).await?;
        sqlx::query(
            "INSERT INTO balance_reservations(payout_id,amount,unit) VALUES(?,?,'KesMinor')",
        )
        .bind(d.operation_id().as_str())
        .bind(d.reserved().value().to_string())
        .execute(&mut *tx)
        .await?;
        enqueue(
            &mut tx,
            &format!("payout:{}", d.operation_id().as_str()),
            "RECONCILE_PAYOUT",
            d.operation_id(),
            payout.next_check_at(),
        )
        .await?;
        audit(
            &mut tx,
            d.operation_id(),
            "DISPATCH_PREPARED",
            payout.started_at(),
        )
        .await?;
        tx.commit().await?;
        Ok(d.operation_id().clone())
    }
    pub async fn change_payout(
        &self,
        id: &Id,
        version: i64,
        change: PayoutChange,
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.write().await?;
        let data: String = sqlx::query_scalar("SELECT data FROM outbound_payouts WHERE id=?")
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
        let mut p: Payout = decode(&data)?;
        match change {
            PayoutChange::Ambiguous(reason) => p.ambiguous(reason)?,
            PayoutChange::Acknowledge {
                originator,
                conversation,
            } => p.acknowledge(originator, conversation)?,
            PayoutChange::DuplicateObserved => p.duplicate_observed()?,
            PayoutChange::Escalate => p.escalate(now)?,
            PayoutChange::Resolve(evidence) => p.resolve(evidence)?,
        }
        changed(sqlx::query("UPDATE outbound_payouts SET state=?,provider_originator=?,provider_conversation=?,data=?,version=version+1 WHERE id=? AND version=?")
            .bind(state(&p.state())?).bind(p.correlation().provider_originator.as_ref().map(Id::as_str)).bind(p.correlation().provider_conversation.as_ref().map(Id::as_str)).bind(encode(&p)?).bind(id.as_str()).bind(version).execute(&mut *tx).await?.rows_affected())?;
        if matches!(p.state(), OutboundState::Paid | OutboundState::Failed) {
            sqlx::query("UPDATE balance_reservations SET released_at=?,evidence=? WHERE payout_id=? AND released_at IS NULL").bind(time(now)?).bind(encode(&p.final_evidence())?).bind(id.as_str()).execute(&mut *tx).await?;
        }
        audit(&mut tx, id, "PAYOUT_CHANGED", now).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn claim_job(&self, now: Timestamp, lease_seconds: u32) -> Result<Option<JobClaim>> {
        if !(1..=300).contains(&lease_seconds) {
            return Err(Error::InvalidTransition.into());
        }
        let until = Timestamp(
            now.0
                .checked_add(u64::from(lease_seconds))
                .ok_or(Error::Overflow)?,
        );
        let mut tx = self.write().await?;
        let row = sqlx::query("UPDATE durable_jobs SET lease_until=?,generation=generation+1 WHERE id=(SELECT id FROM durable_jobs WHERE completed_at IS NULL AND due_at<=? AND (lease_until IS NULL OR lease_until<=?) ORDER BY due_at,id LIMIT 1) RETURNING id,kind,subject,generation")
            .bind(time(until)?).bind(time(now)?).bind(time(now)?).fetch_optional(&mut *tx).await?;
        let claim = row
            .map(|r| {
                Ok::<_, RepositoryError>(JobClaim {
                    id: Id::new(r.try_get::<String, _>("id")?)?,
                    kind: r.try_get("kind")?,
                    subject: Id::new(r.try_get::<String, _>("subject")?)?,
                    generation: r.try_get("generation")?,
                    lease_until: until,
                })
            })
            .transpose()?;
        tx.commit().await?;
        Ok(claim)
    }
    pub async fn finish_job(
        &self,
        claim: &JobClaim,
        now: Timestamp,
        retry_at: Option<Timestamp>,
    ) -> Result<()> {
        let rows = sqlx::query("UPDATE durable_jobs SET due_at=coalesce(?,due_at),completed_at=?,lease_until=NULL WHERE id=? AND generation=? AND completed_at IS NULL AND lease_until>?")
            .bind(retry_at.map(time).transpose()?).bind(if retry_at.is_none() { Some(time(now)?) } else { None }).bind(claim.id.as_str()).bind(claim.generation).bind(time(now)?).execute(&self.pool).await?.rows_affected();
        if rows != 1 {
            return Err(RepositoryError::LeaseLost);
        }
        Ok(())
    }
}
async fn audit(
    conn: &mut SqliteConnection,
    subject: &Id,
    action: &str,
    now: Timestamp,
) -> Result<()> {
    sqlx::query("INSERT INTO audit_records(subject,action,created_at) VALUES(?,?,?)")
        .bind(subject.as_str())
        .bind(action)
        .bind(time(now)?)
        .execute(conn)
        .await?;
    Ok(())
}
async fn enqueue(
    conn: &mut SqliteConnection,
    id: &str,
    kind: &str,
    subject: &Id,
    due: Timestamp,
) -> Result<()> {
    Id::new(id)?;
    sqlx::query("INSERT INTO durable_jobs(id,kind,subject,due_at) VALUES(?,?,?,?) ON CONFLICT(kind,subject) DO NOTHING").bind(id).bind(kind).bind(subject.as_str()).bind(time(due)?).execute(conn).await?;
    Ok(())
}
async fn bind_request(
    conn: &mut SqliteConnection,
    key: &RequestKey,
    payload: &RequestPayload,
    id: &Id,
) -> Result<()> {
    sqlx::query("INSERT INTO request_bindings(caller,operation,request_key,payload_hash,operation_id) VALUES(?,?,?,?,?)").bind(key.caller.as_str()).bind(key.operation.as_str()).bind(key.key.as_str()).bind(payload.hash().as_bytes().as_slice()).bind(id.as_str()).execute(conn).await?;
    Ok(())
}
async fn load_accounting(conn: &mut SqliteConnection, id: Option<&Id>) -> Result<Accounting> {
    let mut a = Accounting::default();
    if let Some(id) = id {
        let data: String = sqlx::query_scalar("SELECT data FROM inbound_quotes WHERE id=?")
            .bind(id.as_str())
            .fetch_optional(&mut *conn)
            .await?
            .ok_or(Error::NotFound)?;
        let quote: InboundQuote = decode(&data)?;
        a.quotes.insert(id.clone(), quote);
        let rows = sqlx::query("SELECT receipt,data FROM mpesa_receipts_ledger WHERE quote_id=?")
            .bind(id.as_str())
            .fetch_all(&mut *conn)
            .await?;
        for row in rows {
            a.receipts.insert(
                crate::identity::ReceiptId::new(row.try_get("receipt")?)?,
                decode(row.try_get("data")?)?,
            );
        }
    }
    Ok(a)
}
async fn save_quote(conn: &mut SqliteConnection, quote: &InboundQuote, version: i64) -> Result<()> {
    changed(
        sqlx::query(
            "UPDATE inbound_quotes SET state=?,data=?,version=version+1 WHERE id=? AND version=?",
        )
        .bind(state(&quote.state())?)
        .bind(encode(quote)?)
        .bind(quote.id().as_str())
        .bind(version)
        .execute(conn)
        .await?
        .rows_affected(),
    )
}
async fn persist_accounting(
    conn: &mut SqliteConnection,
    a: &Accounting,
    id: &Id,
    version: i64,
) -> Result<()> {
    let q = a.quote(id).ok_or(Error::NotFound)?;
    save_quote(conn, q, version).await?;
    for (receipt, r) in &a.receipts {
        sqlx::query("UPDATE mpesa_receipts_ledger SET state=?,data=? WHERE receipt=?")
            .bind(state(&r.state)?)
            .bind(encode(r)?)
            .bind(receipt.as_str())
            .execute(&mut *conn)
            .await?;
    }
    if let Some(event) = a.events().get(id) {
        sqlx::query("INSERT INTO settlement_events(quote_id,created_at,data) VALUES(?,?,?)")
            .bind(id.as_str())
            .bind(time(event.settled_at)?)
            .bind(encode(event)?)
            .execute(&mut *conn)
            .await?;
        sqlx::query("UPDATE unreconciled_deposits SET resolved_at=?,resolution_evidence='QUOTE_SETTLED' WHERE receipt IN (SELECT receipt FROM mpesa_receipts_ledger WHERE quote_id=?) AND resolved_at IS NULL").bind(time(event.settled_at)?).bind(id.as_str()).execute(&mut *conn).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_pool_connection_has_deliberate_durability_settings() {
        let dir = tempfile::tempdir().unwrap();
        let r = Repository::open(dir.path().join("settings.sqlite"))
            .await
            .unwrap();
        let mut connections = Vec::new();
        for _ in 0..4 {
            connections.push(r.pool.acquire().await.unwrap());
        }
        for mut c in connections {
            assert_eq!(
                sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
                    .fetch_one(&mut *c)
                    .await
                    .unwrap(),
                1
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>("PRAGMA synchronous")
                    .fetch_one(&mut *c)
                    .await
                    .unwrap(),
                2
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>("PRAGMA busy_timeout")
                    .fetch_one(&mut *c)
                    .await
                    .unwrap(),
                5000
            );
            assert_eq!(
                sqlx::query_scalar::<_, String>("PRAGMA journal_mode")
                    .fetch_one(&mut *c)
                    .await
                    .unwrap(),
                "wal"
            );
        }
        r.close().await;
    }
}

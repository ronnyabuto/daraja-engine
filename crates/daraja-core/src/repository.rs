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
    ReferenceCollision,
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
mod dispatch;
mod exceptions;
mod liquidity;
mod operations;
mod outbound_reconcile;
mod valuation;
pub use exceptions::{DepositPolicy, RefundAuthorization, RefundRequest};

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

#[derive(Clone)]
pub enum StkChange {
    Start(StkAttempt),
    Acknowledge {
        attempt: Id,
        merchant: Id,
        checkout: Id,
    },
    /// Bind late dispatch metadata without regressing a paid quote or terminal attempt.
    RecordAcknowledgment {
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
    pub poll_slot: i64,
    pub retry_count: i64,
}

impl Repository {
    pub(crate) async fn callback_quote(
        &self,
        binding: &str,
        stk: bool,
    ) -> Result<Option<InboundQuote>> {
        let sql = if stk {
            "SELECT q.data FROM inbound_quotes q JOIN stk_attempts a ON a.quote_id=q.id WHERE a.checkout_request=?"
        } else {
            "SELECT data FROM inbound_quotes WHERE account_ref=?"
        };
        let data: Option<String> = sqlx::query_scalar(sql)
            .bind(binding)
            .fetch_optional(&self.pool)
            .await?;
        data.map(|s| decode(&s)).transpose()
    }

    /// Local, consistent snapshot only; a validation response never reserves or settles funds.
    pub(crate) async fn c2b_expected(
        &self,
        account: &str,
        shortcode: &Id,
        now: Timestamp,
        topups: bool,
    ) -> Result<Option<crate::amount::Amount>> {
        use crate::{
            amount::{Amount, Unit},
            inbound::InboundState,
        };
        let mut tx = self.pool.begin().await?;
        let data: Option<String> =
            sqlx::query_scalar("SELECT data FROM inbound_quotes WHERE account_ref=?")
                .bind(account)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(data) = data else { return Ok(None) };
        let quote: InboundQuote = decode(&data)?;
        if quote.shortcode() != shortcode
            || !quote
                .accepts_receipt_time(now, exceptions::policy(&mut tx).await?.kes_grace_seconds)?
            || quote
                .terms()
                .fx
                .as_ref()
                .is_some_and(|fx| now >= fx.expires_at)
        {
            return Ok(None);
        }
        let expected = match quote.state() {
            InboundState::Issued | InboundState::StkSent => Some(quote.terms().expected),
            InboundState::UnderpaidHeld if topups => {
                let rows: Vec<String> =
                    sqlx::query_scalar("SELECT data FROM mpesa_receipts_ledger WHERE quote_id=?")
                        .bind(quote.id().as_str())
                        .fetch_all(&mut *tx)
                        .await?;
                let mut total = Amount::new(0, Unit::KesMinor);
                for row in rows {
                    let receipt: ConfirmedReceipt = decode(&row)?;
                    total = total.checked_add(receipt.deposit.evidence().amount)?;
                }
                let remaining = quote.terms().expected.checked_sub(total)?;
                (remaining.value() > 0).then_some(remaining)
            }
            _ => None,
        };
        tx.commit().await?;
        Ok(expected)
    }

    pub(crate) async fn accept_callback(
        &self,
        endpoint: &str,
        quote: Option<&Id>,
        body: &str,
        now: Timestamp,
    ) -> Result<()> {
        use sha2::{Digest, Sha256};
        let canonical =
            crate::transport::strict_json(body).map_err(|_| RepositoryError::Encoding)?;
        let canonical = encode(&canonical)?;
        let mut hash = Sha256::new();
        hash.update(b"daraja.callback.v1\0");
        hash.update(endpoint.as_bytes());
        hash.update([0]);
        hash.update(canonical.as_bytes());
        let id = format!("{:x}", hash.finalize());
        let mut tx = self.write().await?;
        sqlx::query("INSERT INTO callback_inbox(id,endpoint,quote_id,received_at,body) VALUES(?,?,?,?,?) ON CONFLICT(id) DO NOTHING")
            .bind(&id).bind(endpoint).bind(quote.map(Id::as_str)).bind(time(now)?).bind(canonical)
            .execute(&mut *tx).await?;
        sqlx::query("INSERT INTO callback_jobs(observation_id,due_at) VALUES(?,?) ON CONFLICT(observation_id) DO NOTHING")
            .bind(&id).bind(time(now)?).execute(&mut *tx).await?;
        if matches!(
            endpoint,
            "/callbacks/b2c/{secret}"
                | "/callbacks/b2b/{secret}"
                | "/callbacks/b2c/timeout/{secret}"
                | "/callbacks/b2b/timeout/{secret}"
        ) {
            dispatch::link_callbacks(&mut tx, now, None, Some(&id)).await?;
        }
        if matches!(
            endpoint,
            "/callbacks/stk/{secret}" | "/callbacks/c2b/confirmation/{secret}"
        ) {
            enqueue(
                &mut tx,
                &format!("callback:{id}"),
                "INBOUND_CALLBACK",
                &Id::new(id)?,
                now,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

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
    /// Atomically binds a checkout request, quote and optional first/resend attempt.
    pub(crate) async fn prepare_checkout(
        &self,
        key: &RequestKey,
        payload: &RequestPayload,
        quote: &InboundQuote,
        attempt: Option<StkAttempt>,
        version: Option<i64>,
        now: Timestamp,
    ) -> Result<(Id, bool)> {
        if key.operation != payload.operation {
            return Err(Error::EvidenceMismatch.into());
        }
        let mut tx = self.write().await?;
        if let Some(id) = checkout_binding(&mut tx, key, payload).await? {
            return Ok((id, false));
        }
        let mut q = if let Some(version) = version {
            let row = sqlx::query("SELECT data,version FROM inbound_quotes WHERE id=?")
                .bind(quote.id().as_str())
                .fetch_one(&mut *tx)
                .await?;
            if row.try_get::<i64, _>("version")? != version {
                return Err(RepositoryError::Conflict);
            }
            decode::<InboundQuote>(row.try_get("data")?)?
        } else {
            let fresh = InboundQuote::new(
                quote.id().clone(),
                quote.account_ref().clone(),
                quote.shortcode().clone(),
                quote.terms().clone(),
            )?;
            if fresh != *quote {
                return Err(Error::InvalidTransition.into());
            }
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM inbound_quotes WHERE account_ref=?)",
            )
            .bind(quote.account_ref().as_str())
            .fetch_one(&mut *tx)
            .await?;
            if exists {
                return Err(RepositoryError::ReferenceCollision);
            }
            sqlx::query("INSERT INTO inbound_quotes(id,account_ref,state,data) VALUES(?,?,?,?)")
                .bind(quote.id().as_str())
                .bind(quote.account_ref().as_str())
                .bind(state(&quote.state())?)
                .bind(encode(quote)?)
                .execute(&mut *tx)
                .await?;
            quote.clone()
        };
        if let Some(a) = attempt {
            q.start_stk(a.clone())?;
            save_quote(&mut tx, &q, version.unwrap_or(0)).await?;
            persist_attempt(&mut tx, q.id(), &a).await?;
        }
        bind_request(&mut tx, key, payload, q.id()).await?;
        audit(&mut tx, q.id(), "CHECKOUT_PREPARED", now).await?;
        tx.commit().await?;
        Ok((q.id().clone(), true))
    }

    pub(crate) async fn checkout_binding(
        &self,
        key: &RequestKey,
        payload: &RequestPayload,
    ) -> Result<Option<Id>> {
        let mut conn = self.pool.acquire().await?;
        checkout_binding(&mut conn, key, payload).await
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
            StkChange::RecordAcknowledgment {
                attempt,
                merchant,
                checkout,
            } => q.record_stk_ack(&attempt, merchant, checkout)?,
            StkChange::Unknown(a) => q.stk_unknown(&a)?,
            StkChange::Finish { attempt, succeeded } => q.finish_stk(&attempt, succeeded)?,
        }
        save_quote(&mut tx, q, version).await?;
        for a in q.attempts() {
            persist_attempt(&mut tx, id, a).await?;
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
        self.confirm_inner(deposit, settle_at, None).await
    }
    pub(crate) async fn apply_inbound_evidence(
        &self,
        claim: &JobClaim,
        deposit: VerifiedDeposit,
        now: Timestamp,
    ) -> Result<DepositState> {
        self.confirm_inner(deposit, Some(now), Some(claim)).await
    }
    async fn confirm_inner(
        &self,
        deposit: VerifiedDeposit,
        settle_at: Option<Timestamp>,
        worker: Option<&JobClaim>,
    ) -> Result<DepositState> {
        let mut tx = self.write().await?;
        if let Some(claim) = worker {
            check_lease(&mut tx, claim, settle_at.ok_or(Error::InvalidTransition)?).await?;
        }
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
            if let Some(claim) = worker {
                let outcome = if let Some(r) = a.receipt(&receipt) {
                    r.state
                } else {
                    let data: String = sqlx::query_scalar(
                        "SELECT data FROM mpesa_receipts_ledger WHERE receipt=?",
                    )
                    .bind(receipt.as_str())
                    .fetch_one(&mut *tx)
                    .await?;
                    decode::<ConfirmedReceipt>(&data)?.state
                };
                finish_inbound(
                    &mut tx,
                    claim,
                    settle_at.ok_or(Error::InvalidTransition)?,
                    None,
                    "DUPLICATE",
                    false,
                    false,
                )
                .await?;
                tx.commit().await?;
                return Ok(outcome);
            }
            return Err(Error::DuplicateReceipt.into());
        }
        let policy = exceptions::policy(&mut tx).await?;
        exceptions::apply_risk(&mut tx, &mut a, &deposit, &policy).await?;
        a.confirm_with_grace(deposit, policy.kes_grace_seconds)?;
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
                let q = a.quote(id).ok_or(Error::NotFound)?;
                if worker.is_none()
                    || (matches!(
                        q.state(),
                        crate::inbound::InboundState::Verifying
                            | crate::inbound::InboundState::SettlementHeld
                    ) && !a.blocked
                        && settle_at >= q.terms().hold_until
                        && q.terms()
                            .fx
                            .as_ref()
                            .is_none_or(|fx| settle_at < fx.expires_at))
                {
                    a.settle(id, settle_at)?;
                }
            }
            persist_accounting(&mut tx, &a, id, version).await?;
        } else if settle_at.is_some() && worker.is_none() {
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
        if let Some(claim) = worker {
            let retry = quote_id
                .as_ref()
                .and_then(|id| a.quote(id))
                .filter(|q| q.state() == crate::inbound::InboundState::SettlementHeld)
                .map(|q| q.terms().hold_until);
            finish_inbound(
                &mut tx,
                claim,
                settle_at.ok_or(Error::InvalidTransition)?,
                retry,
                "VERIFIED",
                false,
                false,
            )
            .await?;
        }
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
    /// A replay returns no send authority. The saga uses a separate insertion claim.
    /// Successful return is the commit-before-I/O crash boundary. Recovery only
    /// schedules reconciliation, never automatic resubmission.
    pub async fn prepare_payout(&self, key: &RequestKey, payout: &Payout) -> Result<Id> {
        Ok(self.prepare_payout_claim(key, payout, None, None).await?.0)
    }
    pub(crate) async fn prepare_payout_claim(
        &self,
        key: &RequestKey,
        payout: &Payout,
        quoted_endpoint: Option<&str>,
        sweep: Option<crate::liquidity::SweepClaim<'_>>,
    ) -> Result<(Id, bool)> {
        let d = payout.dispatch();
        if key.operation != d.payload().operation {
            return Err(Error::EvidenceMismatch.into());
        }
        let mut tx = self.write().await?;
        if let Some(row) = sqlx::query("SELECT payload_hash,operation_id FROM request_bindings WHERE caller=? AND operation=? AND request_key=?")
            .bind(key.caller.as_str()).bind(key.operation.as_str()).bind(key.key.as_str()).fetch_optional(&mut *tx).await? {
            if row.try_get::<Vec<u8>,_>("payload_hash")? != d.hash().as_bytes() { return Err(Error::IdempotencyConflict.into()); }
            let id = Id::new(row.try_get::<String,_>("operation_id")?)?;
            let data: String = sqlx::query_scalar("SELECT data FROM outbound_payouts WHERE id=?")
                .bind(id.as_str()).fetch_one(&mut *tx).await?;
            if decode::<Payout>(&data)?.dispatch().payload() != d.payload() {
                return Err(Error::IdempotencyConflict.into());
            }
            return Ok((id, false));
        }
        // Revalidate deserialized values before admitting a new dispatch.
        let mut validated = crate::outbound::DispatchRecord::new(
            d.operation_id().clone(),
            d.payload().clone(),
            d.wire_body().to_vec(),
            d.tariff().clone(),
            d.reserved(),
        )?;
        if let Some(quote) = d.quote() {
            let row = sqlx::query("SELECT data FROM payment_quotes WHERE id=?")
                .bind(quote.id().as_str())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error::NotFound)?;
            let stored: crate::valuation::PaymentQuote = decode(row.try_get("data")?)?;
            if stored != *quote {
                return Err(Error::EvidenceMismatch.into());
            }
            validated = validated.with_quote(stored, payout.started_at())?;
        }
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
        if let Some(endpoint) = quoted_endpoint {
            if d.quote().is_none() && sweep.is_none() {
                return Err(Error::EvidenceMismatch.into());
            }
            sqlx::query("INSERT INTO payout_dispatches(payout_id,quote_id,command,endpoint) VALUES(?,?,?,?)")
                .bind(d.operation_id().as_str())
                .bind(d.quote().map(|q| q.id().as_str()))
                .bind(
                    serde_json::from_slice::<serde_json::Value>(d.wire_body())?["CommandID"]
                        .as_str()
                        .ok_or(Error::EvidenceMismatch)?,
                )
                .bind(endpoint)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(sweep) = sweep {
            liquidity::admit_sweep(&mut tx, payout, sweep).await?;
        } else {
            liquidity::admit_payout(&mut tx, payout).await?;
        }
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
        Ok((d.operation_id().clone(), true))
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
            PayoutChange::Escalate => {
                p.escalate(now)?;
                sqlx::query("INSERT INTO payout_alerts(payout_id,reason,created_at) VALUES(?,'MAX_PENDING_TTL',?) ON CONFLICT DO NOTHING")
                    .bind(id.as_str()).bind(time(now)?).execute(&mut *tx).await?;
            }
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
        self.claim_job_kind(now, lease_seconds, false).await
    }
    pub(crate) async fn claim_inbound_job(
        &self,
        now: Timestamp,
        lease_seconds: u32,
    ) -> Result<Option<JobClaim>> {
        self.claim_job_kind(now, lease_seconds, true).await
    }
    async fn claim_job_kind(
        &self,
        now: Timestamp,
        lease_seconds: u32,
        inbound_only: bool,
    ) -> Result<Option<JobClaim>> {
        if !(1..=300).contains(&lease_seconds) {
            return Err(Error::InvalidTransition.into());
        }
        let until = Timestamp(
            now.0
                .checked_add(u64::from(lease_seconds))
                .ok_or(Error::Overflow)?,
        );
        let mut tx = self.write().await?;
        let row = sqlx::query("UPDATE durable_jobs SET lease_until=?,generation=generation+1 WHERE id=(SELECT id FROM durable_jobs WHERE completed_at IS NULL AND due_at<=? AND (lease_until IS NULL OR lease_until<=?) AND (?=0 OR kind IN ('POLL_STK','INBOUND_CALLBACK','VERIFY_RECEIPT')) ORDER BY due_at,id LIMIT 1) RETURNING id,kind,subject,generation,poll_slot,retry_count")
            .bind(time(until)?).bind(time(now)?).bind(time(now)?).bind(inbound_only).fetch_optional(&mut *tx).await?;
        let claim = row
            .map(|r| {
                Ok::<_, RepositoryError>(JobClaim {
                    id: Id::new(r.try_get::<String, _>("id")?)?,
                    kind: r.try_get("kind")?,
                    subject: Id::new(r.try_get::<String, _>("subject")?)?,
                    generation: r.try_get("generation")?,
                    lease_until: until,
                    poll_slot: r.try_get("poll_slot")?,
                    retry_count: r.try_get("retry_count")?,
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
    pub(crate) async fn settle_inbound_hold(
        &self,
        claim: &JobClaim,
        id: &Id,
        now: Timestamp,
    ) -> Result<bool> {
        let mut tx = self.write().await?;
        check_lease(&mut tx, claim, now).await?;
        let mut a = load_accounting(&mut tx, Some(id)).await?;
        let q = a.quote(id).ok_or(Error::NotFound)?;
        if a.blocked
            || !matches!(
                q.state(),
                crate::inbound::InboundState::SettlementHeld
                    | crate::inbound::InboundState::Verifying
            )
            || now < q.terms().hold_until
            || q.terms().fx.as_ref().is_some_and(|fx| now >= fx.expires_at)
        {
            return Ok(false);
        }
        let version: i64 = sqlx::query_scalar("SELECT version FROM inbound_quotes WHERE id=?")
            .bind(id.as_str())
            .fetch_one(&mut *tx)
            .await?;
        a.settle(id, now)?;
        persist_accounting(&mut tx, &a, id, version).await?;
        finish_inbound(
            &mut tx,
            claim,
            now,
            None,
            "VERIFIED_HOLD_RELEASED",
            false,
            false,
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }
    pub(crate) async fn finish_inbound_job(
        &self,
        claim: &JobClaim,
        now: Timestamp,
        retry: Option<Timestamp>,
        reason: &str,
        advance: bool,
        throttled: bool,
    ) -> Result<()> {
        let mut tx = self.write().await?;
        finish_inbound(&mut tx, claim, now, retry, reason, advance, throttled).await?;
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn inbound_callback(
        &self,
        id: &Id,
    ) -> Result<(String, Option<Id>, crate::crypto::Secret, Timestamp)> {
        let row =
            sqlx::query("SELECT endpoint,quote_id,body,received_at FROM callback_inbox WHERE id=?")
                .bind(id.as_str())
                .fetch_one(&self.pool)
                .await?;
        Ok((
            row.try_get("endpoint")?,
            row.try_get::<Option<String>, _>("quote_id")?
                .map(Id::new)
                .transpose()?,
            crate::crypto::Secret::new(row.try_get::<String, _>("body")?),
            Timestamp(row.try_get::<i64, _>("received_at")? as u64),
        ))
    }
    pub(crate) async fn status_prepared(
        &self,
        id: &Id,
        receipt: &crate::identity::ReceiptId,
        now: Timestamp,
    ) -> Result<bool> {
        let mut tx = self.write().await?;
        let inserted = sqlx::query("INSERT INTO inbound_status_queries(id,receipt,created_at) VALUES(?,?,?) ON CONFLICT(id) DO NOTHING").bind(id.as_str()).bind(receipt.as_str()).bind(time(now)?).execute(&mut *tx).await?.rows_affected() == 1;
        let old: String =
            sqlx::query_scalar("SELECT receipt FROM inbound_status_queries WHERE id=?")
                .bind(id.as_str())
                .fetch_one(&mut *tx)
                .await?;
        if old != receipt.as_str() {
            return Err(Error::EvidenceMismatch.into());
        }
        tx.commit().await?;
        Ok(inserted)
    }
    pub(crate) async fn status_ack(
        &self,
        id: &Id,
        originator: &Id,
        conversation: &Id,
    ) -> Result<()> {
        changed(sqlx::query("UPDATE inbound_status_queries SET originator=?,conversation=? WHERE id=? AND (originator IS NULL OR originator=?) AND (conversation IS NULL OR conversation=?)").bind(originator.as_str()).bind(conversation.as_str()).bind(id.as_str()).bind(originator.as_str()).bind(conversation.as_str()).execute(&self.pool).await?.rows_affected())
    }
    pub(crate) async fn status_results(&self, id: &Id) -> Result<Vec<crate::crypto::Secret>> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT i.body FROM callback_inbox i JOIN inbound_status_queries q ON json_extract(i.body,'$.Result.OriginatorConversationID')=q.originator AND json_extract(i.body,'$.Result.ConversationID')=q.conversation WHERE q.id=? AND i.endpoint='/callbacks/status/result/{secret}' ORDER BY i.received_at,i.id LIMIT 100").bind(id.as_str()).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(crate::crypto::Secret::new).collect())
    }
}
async fn check_lease(conn: &mut SqliteConnection, claim: &JobClaim, now: Timestamp) -> Result<()> {
    let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM durable_jobs WHERE id=? AND generation=? AND completed_at IS NULL AND lease_until>?)").bind(claim.id.as_str()).bind(claim.generation).bind(time(now)?).fetch_one(conn).await?;
    if !valid {
        return Err(RepositoryError::LeaseLost);
    }
    Ok(())
}
async fn finish_inbound(
    conn: &mut SqliteConnection,
    claim: &JobClaim,
    now: Timestamp,
    retry: Option<Timestamp>,
    reason: &str,
    advance: bool,
    throttled: bool,
) -> Result<()> {
    check_lease(conn, claim, now).await?;
    sqlx::query("UPDATE durable_jobs SET due_at=coalesce(?,due_at),completed_at=?,lease_until=NULL,poll_slot=poll_slot+?,retry_count=?,last_reason=? WHERE id=? AND generation=?")
        .bind(retry.map(time).transpose()?).bind(if retry.is_none() {Some(time(now)?)} else {None}).bind(i64::from(advance)).bind(if throttled {claim.retry_count.saturating_add(1)} else {0}).bind(reason).bind(claim.id.as_str()).bind(claim.generation).execute(&mut *conn).await?;
    if claim.kind == "INBOUND_CALLBACK" {
        sqlx::query("UPDATE callback_jobs SET due_at=coalesce(?,due_at),completed_at=? WHERE observation_id=?")
            .bind(retry.map(time).transpose()?).bind(if retry.is_none() {Some(time(now)?)} else {None}).bind(claim.subject.as_str()).execute(conn).await?;
    }
    Ok(())
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
async fn checkout_binding(
    conn: &mut SqliteConnection,
    key: &RequestKey,
    payload: &RequestPayload,
) -> Result<Option<Id>> {
    let row = sqlx::query("SELECT payload_hash,operation_id FROM request_bindings WHERE caller=? AND operation=? AND request_key=?")
        .bind(key.caller.as_str()).bind(key.operation.as_str()).bind(key.key.as_str()).fetch_optional(&mut *conn).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let operation_id = Id::new(row.try_get::<String, _>("operation_id")?)?;
    let stored_hash = row.try_get::<Vec<u8>, _>("payload_hash")?;
    if stored_hash == payload.hash().as_bytes() {
        return Ok(Some(operation_id));
    }
    // Stage 08 bindings included operator configuration. Reconstruct those exact
    // terms from the frozen quote, without repricing or replacing the binding.
    if payload
        .options
        .get("request_contract")
        .is_some_and(|v| v == "inbound-v2")
    {
        let row = sqlx::query("SELECT q.data, MIN(a.created_at) AS created_at FROM inbound_quotes q JOIN audit_records a ON a.subject=q.id AND a.action='CHECKOUT_PREPARED' WHERE q.id=? GROUP BY q.id")
            .bind(operation_id.as_str()).fetch_optional(&mut *conn).await?;
        if let Some(row) = row {
            let q: InboundQuote = decode(row.try_get("data")?)?;
            let created =
                u64::try_from(row.try_get::<i64, _>("created_at")?).map_err(|_| Error::Overflow)?;
            let terms = q.terms();
            if let (Some(ttl), Some(hold)) = (
                terms.quote_expires_at.0.checked_sub(created),
                terms.hold_until.0.checked_sub(created),
            ) {
                let mut legacy = payload.clone();
                legacy.tariff_version = terms.tariff.version.clone();
                legacy.options.remove("request_contract");
                legacy.options.insert("ttl".into(), ttl.to_string());
                legacy.options.insert("hold".into(), hold.to_string());
                legacy
                    .options
                    .insert("fee_minor".into(), terms.tariff.fee.value().to_string());
                if stored_hash == legacy.hash().as_bytes() {
                    return Ok(Some(operation_id));
                }
            }
        }
    }
    Err(Error::IdempotencyConflict.into())
}

async fn persist_attempt(conn: &mut SqliteConnection, quote: &Id, a: &StkAttempt) -> Result<()> {
    sqlx::query("INSERT INTO stk_attempts(id,quote_id,caller_request,merchant_request,checkout_request,next_poll_at,resend_after,data) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET merchant_request=excluded.merchant_request,checkout_request=excluded.checkout_request,data=excluded.data")
        .bind(a.id().as_str()).bind(quote.as_str()).bind(a.caller_request().as_str()).bind(a.merchant_request().map(Id::as_str)).bind(a.checkout_request().map(Id::as_str)).bind(time(a.next_poll_at())?).bind(time(a.resend_after())?).bind(encode(a)?).execute(&mut *conn).await?;
    enqueue(
        conn,
        &format!("stk:{}", a.id().as_str()),
        "POLL_STK",
        a.id(),
        a.next_poll_at(),
    )
    .await
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
        a.blocked =
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM deposit_blocks WHERE quote_id=?")
                .bind(id.as_str())
                .fetch_one(&mut *conn)
                .await?
                != 0;
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

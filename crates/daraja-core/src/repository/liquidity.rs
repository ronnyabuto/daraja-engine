use super::*;
use crate::{
    amount::{Amount, Unit},
    collection::StatusAcknowledgment,
    crypto::Secret,
    liquidity::{AccountRole, BalanceEvidence, BalanceTarget, FloatStatus, LiquidityPolicy},
    payout::{BalanceAccount, BalancePrepared},
};

fn minor(value: &str) -> Result<Amount> {
    Ok(Amount::new(
        value.parse().map_err(|_| RepositoryError::Encoding)?,
        Unit::KesMinor,
    ))
}
impl Repository {
    /// Accounts cannot be rebound: existing reservations belong to these ledgers.
    pub async fn configure_liquidity(&self, policy: LiquidityPolicy) -> Result<()> {
        policy.validate()?;
        let mut tx = self.write().await?;
        if let Some(row) =
            sqlx::query("SELECT collection,disbursement FROM liquidity_policy WHERE singleton=1")
                .fetch_optional(&mut *tx)
                .await?
            && (row.try_get::<String, _>("collection")? != policy.collection.as_str()
                || row.try_get::<String, _>("disbursement")? != policy.disbursement.as_str())
        {
            return Err(Error::EvidenceMismatch.into());
        }
        sqlx::query("INSERT INTO liquidity_policy VALUES(1,?,?,?,?,?) ON CONFLICT(singleton) DO UPDATE SET minimum=excluded.minimum,max_age=excluded.max_age,poll_interval=excluded.poll_interval")
            .bind(policy.collection.as_str()).bind(policy.disbursement.as_str()).bind(policy.minimum.value().to_string())
            .bind(policy.max_age_seconds).bind(policy.poll_interval_seconds).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn liquidity_policy(&self) -> Result<LiquidityPolicy> {
        let mut tx = self.pool.begin().await?;
        policy(&mut tx).await
    }
    /// Durable immutable query preparation, shared by monitor hooks and trusted adapters.
    pub async fn prepare_balance_query(
        &self,
        role: AccountRole,
        record: BalancePrepared<'_>,
    ) -> Result<()> {
        let now = Timestamp(
            record
                .created_at
                .timestamp()
                .try_into()
                .map_err(|_| Error::Overflow)?,
        );
        let mut tx = self.write().await?;
        let config = policy(&mut tx).await?;
        let account = match &record.request.account {
            BalanceAccount::Shortcode(id) => id,
            BalanceAccount::Till(_) => return Err(Error::EvidenceMismatch.into()),
        };
        let expected = match role {
            AccountRole::Collection => &config.collection,
            AccountRole::Disbursement => &config.disbursement,
        };
        let body = crate::transport::strict_json(record.wire_body.expose())
            .map_err(|_| RepositoryError::Encoding)?;
        if account != expected
            || body["PartyA"].as_str() != Some(account.as_str())
            || body["CommandID"] != "AccountBalance"
            || body["IdentifierType"] != 4
            || body["ResultURL"].as_str() != Some(record.request.result_url.expose())
            || body["QueueTimeOutURL"].as_str() != Some(record.request.timeout_url.expose())
        {
            return Err(Error::EvidenceMismatch.into());
        }
        let last: Option<i64> =
            sqlx::query_scalar("SELECT max(created_at) FROM balance_queries WHERE role=?")
                .bind(role.name())
                .fetch_one(&mut *tx)
                .await?;
        if last.is_some_and(|last| {
            now.0 < (last as u64).saturating_add(u64::from(config.poll_interval_seconds))
        }) {
            return Err(RepositoryError::Conflict);
        }
        sqlx::query(
            "INSERT INTO balance_queries(id,role,account,created_at,wire_body) VALUES(?,?,?,?,?)",
        )
        .bind(record.request.local_query_id.as_str())
        .bind(role.name())
        .bind(account.as_str())
        .bind(time(now)?)
        .bind(record.wire_body.expose())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn acknowledge_balance(&self, id: &Id, ack: &StatusAcknowledgment) -> Result<()> {
        changed(sqlx::query("UPDATE balance_queries SET originator=?,conversation=? WHERE id=? AND (originator IS NULL OR originator=?) AND (conversation IS NULL OR conversation=?)")
            .bind(ack.originator_conversation_id.as_str()).bind(ack.conversation_id.as_str()).bind(id.as_str())
            .bind(ack.originator_conversation_id.as_str()).bind(ack.conversation_id.as_str()).execute(&self.pool).await?.rows_affected())
    }
    pub async fn balance_targets(&self, role: AccountRole) -> Result<Vec<BalanceTarget>> {
        let mut tx = self.pool.begin().await?;
        // Bounded recent work; historical observations remain in the database for audit.
        let rows = sqlx::query("SELECT id,account,created_at,originator,conversation FROM balance_queries WHERE role=? AND applied_at IS NULL AND originator IS NOT NULL ORDER BY sequence DESC LIMIT 100")
            .bind(role.name()).fetch_all(&mut *tx).await?;
        let mut targets = Vec::new();
        for row in rows {
            let callbacks: Vec<String> = sqlx::query_scalar("SELECT body FROM callback_inbox WHERE endpoint IN ('/callbacks/balance/result/{secret}','/callbacks/balance/timeout/{secret}') AND json_extract(body,'$.Result.OriginatorConversationID')=? AND json_extract(body,'$.Result.ConversationID')=? ORDER BY received_at,id LIMIT 100")
                .bind(row.try_get::<String,_>("originator")?).bind(row.try_get::<String,_>("conversation")?).fetch_all(&mut *tx).await?;
            targets.push(BalanceTarget {
                query_id: Id::new(row.try_get::<String, _>("id")?)?,
                role,
                account: Id::new(row.try_get::<String, _>("account")?)?,
                requested_at: Timestamp(
                    row.try_get::<i64, _>("created_at")?
                        .try_into()
                        .map_err(|_| Error::Overflow)?,
                ),
                callbacks: callbacks.into_iter().map(Secret::new).collect(),
            });
        }
        Ok(targets)
    }
    /// Trusted evidence boundary. A callback must not call this directly.
    /// Request time is the conservative age/causal watermark, never callback arrival time.
    pub async fn record_verified_balance(
        &self,
        query: &Id,
        evidence: BalanceEvidence,
        now: Timestamp,
    ) -> Result<()> {
        evidence.available.require_kes()?;
        let mut tx = self.write().await?;
        let row = sqlx::query("SELECT role,account,created_at,originator,amount,evidence FROM balance_queries WHERE id=?")
            .bind(query.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        if evidence.query_id != *query
            || row.try_get::<String, _>("role")? != evidence.role.name()
            || row.try_get::<String, _>("account")? != evidence.account.as_str()
            || row.try_get::<i64, _>("created_at")? > time(now)?
            || row.try_get::<Option<String>, _>("originator")?.is_none()
        {
            return Err(Error::EvidenceMismatch.into());
        }
        if let Some(amount) = row.try_get::<Option<String>, _>("amount")? {
            if minor(&amount)? != evidence.available
                || row.try_get::<Option<String>, _>("evidence")?.as_deref()
                    != Some(evidence.evidence_record.as_str())
            {
                return Err(Error::IdempotencyConflict.into());
            }
            return Ok(());
        }
        sqlx::query("UPDATE balance_queries SET amount=?,evidence=?,applied_at=? WHERE id=?")
            .bind(evidence.available.value().to_string())
            .bind(evidence.evidence_record.as_str())
            .bind(time(now)?)
            .bind(query.as_str())
            .execute(&mut *tx)
            .await?;
        audit(&mut tx, query, "VERIFIED_BALANCE", now).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn float_status(&self, role: AccountRole, now: Timestamp) -> Result<FloatStatus> {
        let mut tx = self.pool.begin().await?;
        status(&mut tx, role, now).await
    }
    pub async fn refresh_liquidity_alerts(&self, now: Timestamp) -> Result<()> {
        let mut tx = self.write().await?;
        for role in [AccountRole::Collection, AccountRole::Disbursement] {
            let s = status(&mut tx, role, now).await?;
            let reason = if s.available.is_none() {
                "BALANCE_UNAVAILABLE_OR_STALE"
            } else {
                "LOW_FLOAT"
            };
            sqlx::query("INSERT INTO liquidity_alerts(role,reason,active,updated_at) VALUES(?,?,?,?) ON CONFLICT(role) DO UPDATE SET reason=excluded.reason,active=excluded.active,updated_at=excluded.updated_at")
                .bind(role.name()).bind(reason).bind(s.alert).bind(time(now)?).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    /// Prometheus text exposition; label values are a fixed two-element account-role set.
    pub async fn liquidity_metrics(&self, now: Timestamp) -> Result<String> {
        let mut tx = self.pool.begin().await?;
        let mut output = String::new();
        for role in [AccountRole::Collection, AccountRole::Disbursement] {
            let s = status(&mut tx, role, now).await?;
            let balance_name = match role {
                AccountRole::Collection => "daraja_c2b_collection_balance_kes",
                AccountRole::Disbursement => "daraja_b2c_utility_balance_kes",
            };
            if let Some(balance) = s.balance {
                output.push_str(&format!(
                    "{balance_name} {}.{:02}\n",
                    balance.value() / 100,
                    balance.value() % 100
                ));
            }
            let age = s
                .age_seconds
                .map_or_else(|| "+Inf".to_owned(), |a| a.to_string());
            output.push_str(&format!(
                "daraja_balance_age_seconds{{account=\"{}\"}} {age}\n",
                role.name()
            ));
            for (name, value) in [
                ("pending_liability", s.pending_liability),
                ("quote_reservations", s.quote_reservations),
            ] {
                output.push_str(&format!(
                    "daraja_{name}_kes{{account=\"{}\"}} {}.{:02}\n",
                    role.name(),
                    value.value() / 100,
                    value.value() % 100
                ));
            }
            output.push_str(&format!(
                "daraja_liquidity_alert{{account=\"{}\"}} {}\n",
                role.name(),
                u8::from(s.alert)
            ));
        }
        Ok(output)
    }
}
async fn policy(tx: &mut Transaction<'_, Sqlite>) -> Result<LiquidityPolicy> {
    let row = sqlx::query("SELECT * FROM liquidity_policy WHERE singleton=1")
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(Error::LiquidityUnavailable)?;
    Ok(LiquidityPolicy {
        collection: Id::new(row.try_get::<String, _>("collection")?)?,
        disbursement: Id::new(row.try_get::<String, _>("disbursement")?)?,
        minimum: minor(row.try_get("minimum")?)?,
        max_age_seconds: row
            .try_get::<i64, _>("max_age")?
            .try_into()
            .map_err(|_| Error::Overflow)?,
        poll_interval_seconds: row
            .try_get::<i64, _>("poll_interval")?
            .try_into()
            .map_err(|_| Error::Overflow)?,
    })
}
async fn status(
    tx: &mut Transaction<'_, Sqlite>,
    role: AccountRole,
    now: Timestamp,
) -> Result<FloatStatus> {
    let config = policy(tx).await?;
    let row = sqlx::query("SELECT amount,created_at FROM balance_queries WHERE role=? AND applied_at IS NOT NULL ORDER BY sequence DESC LIMIT 1")
        .bind(role.name()).fetch_optional(&mut **tx).await?;
    let balance = row
        .as_ref()
        .map(|r| minor(r.try_get("amount")?))
        .transpose()?;
    let requested = row
        .as_ref()
        .map(|r| r.try_get::<i64, _>("created_at"))
        .transpose()?;
    let age = requested.and_then(|t| now.0.checked_sub(t as u64));
    let fresh = age.is_some_and(|age| age < u64::from(config.max_age_seconds));
    let mut liability = Amount::new(0, Unit::KesMinor);
    let mut reservations = liability;
    // Pending may already be reflected in a snapshot: double-counting is conservative.
    // Paid after query start still consumes the older snapshot even after reservation release.
    let rows: Vec<String> = sqlx::query_scalar("SELECT r.amount FROM balance_reservations r JOIN outbound_payouts p ON p.id=r.payout_id LEFT JOIN float_sweeps s ON s.payout_id=p.id WHERE (CASE WHEN s.payout_id IS NULL THEN 'disbursement' ELSE 'collection' END)=? AND (p.state NOT IN ('Paid','Failed') OR (p.state='Paid' AND (r.released_at IS NULL OR r.released_at>=?)))")
        .bind(role.name()).bind(requested.unwrap_or(0)).fetch_all(&mut **tx).await?;
    for value in rows {
        liability = liability.checked_add(minor(&value)?)?;
    }
    if role == AccountRole::Disbursement {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT amount FROM quote_float_reservations WHERE payout_id IS NULL AND expires_at>?",
        )
        .bind(time(now)?)
        .fetch_all(&mut **tx)
        .await?;
        for value in rows {
            reservations = reservations.checked_add(minor(&value)?)?;
        }
    }
    let available = if fresh {
        balance.map(|b| {
            Amount::new(
                b.value()
                    .saturating_sub(liability.value())
                    .saturating_sub(reservations.value()),
                Unit::KesMinor,
            )
        })
    } else {
        None
    };
    let alert = available.is_none_or(|a| a.value() < config.minimum.value());
    Ok(FloatStatus {
        balance,
        age_seconds: age,
        pending_liability: liability,
        quote_reservations: reservations,
        available,
        alert,
    })
}
pub(super) async fn reserve_quote(
    tx: &mut Transaction<'_, Sqlite>,
    quote: &crate::valuation::PaymentQuote,
    now: Timestamp,
) -> Result<()> {
    let amount = quote.principal().checked_add(quote.tariff().fee)?;
    ensure_capacity(tx, amount, now).await?;
    sqlx::query("INSERT INTO quote_float_reservations(quote_id,amount,expires_at) VALUES(?,?,?)")
        .bind(quote.id().as_str())
        .bind(amount.value().to_string())
        .bind(time(quote.expires_at())?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub(super) async fn admit_payout(tx: &mut Transaction<'_, Sqlite>, payout: &Payout) -> Result<()> {
    let d = payout.dispatch();
    let now = payout.started_at();
    if let Some(q) = d.quote() {
        let body: serde_json::Value = serde_json::from_slice(d.wire_body())?;
        if body["PartyA"].as_str() != Some(policy(tx).await?.disbursement.as_str()) {
            return Err(Error::EvidenceMismatch.into());
        }
        // Consume the quote hold in the same transaction, avoiding double-counting it.
        changed(sqlx::query("UPDATE quote_float_reservations SET payout_id=? WHERE quote_id=? AND payout_id IS NULL AND expires_at>?")
            .bind(d.operation_id().as_str()).bind(q.id().as_str()).bind(time(now)?).execute(&mut **tx).await?.rows_affected())?;
    }
    ensure_capacity(tx, d.reserved(), now).await
}
async fn ensure_capacity(
    tx: &mut Transaction<'_, Sqlite>,
    required: Amount,
    now: Timestamp,
) -> Result<()> {
    let minimum = policy(tx).await?.minimum;
    let status = status(tx, AccountRole::Disbursement, now).await?;
    let available = status.available.ok_or(Error::LiquidityUnavailable)?;
    if available.value() < minimum.checked_add(required)?.value() {
        return Err(Error::InsufficientFloat.into());
    }
    Ok(())
}

impl Repository {
    pub(crate) async fn sweep_candidate(&self, now: Timestamp) -> Result<Option<Id>> {
        let mut tx = self.pool.begin().await?;
        let utility = status(&mut tx, AccountRole::Disbursement, now).await?;
        let source = status(&mut tx, AccountRole::Collection, now).await?;
        if utility.available.is_none() || !utility.alert || source.available.is_none() {
            return Ok(None);
        }
        let id: String = sqlx::query_scalar("SELECT id FROM balance_queries WHERE role='collection' AND applied_at IS NOT NULL ORDER BY sequence DESC LIMIT 1")
            .fetch_one(&mut *tx).await?;
        Ok(Some(Id::new(id)?))
    }
}
pub(super) async fn admit_sweep(
    tx: &mut Transaction<'_, Sqlite>,
    payout: &Payout,
    claim: crate::liquidity::SweepClaim<'_>,
) -> Result<()> {
    let config = claim.config;
    let policy = policy(tx).await?;
    let now = payout.started_at();
    config.max_daily.require_kes()?;
    let gross = config.amount.checked_add(config.tariff.fee)?;
    if config.source != policy.collection
        || config.utility != policy.disbursement
        || config.cooldown_seconds == 0
        || config.amount.whole_kes()? == 0
        || config.max_daily.value() < gross.value()
    {
        return Err(Error::EvidenceMismatch.into());
    }
    let wire: serde_json::Value = serde_json::from_slice(payout.dispatch().wire_body())?;
    if wire["CommandID"] != "BusinessTransferFromMMFToUtility"
        || wire["PartyA"].as_str() != Some(config.source.as_str())
        || wire["PartyB"].as_str() != Some(config.utility.as_str())
        || wire["Amount"].as_u64() != Some(config.amount.whole_kes()?)
        || payout.dispatch().reserved() != gross
    {
        return Err(Error::EvidenceMismatch.into());
    }
    let snapshot: String = sqlx::query_scalar("SELECT id FROM balance_queries WHERE role='collection' AND applied_at IS NOT NULL ORDER BY sequence DESC LIMIT 1")
        .fetch_optional(&mut **tx).await?.ok_or(Error::LiquidityUnavailable)?;
    if snapshot != claim.snapshot.as_str() {
        return Err(RepositoryError::Conflict);
    }
    let utility = status(tx, AccountRole::Disbursement, now).await?;
    let source = status(tx, AccountRole::Collection, now).await?;
    if utility.available.is_none() || !utility.alert {
        return Err(Error::LiquidityUnavailable.into());
    }
    if source.available.ok_or(Error::LiquidityUnavailable)?.value()
        < policy.minimum.checked_add(gross)?.value()
    {
        return Err(Error::InsufficientFloat.into());
    }
    let unresolved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM float_sweeps s JOIN outbound_payouts p ON p.id=s.payout_id WHERE p.state NOT IN ('Paid','Failed'))")
        .fetch_one(&mut **tx).await?;
    if unresolved {
        return Err(RepositoryError::Conflict);
    }
    // A refreshed source alone cannot justify topping up an old utility observation again.
    let resolved: Option<i64> = sqlx::query_scalar("SELECT max(r.released_at) FROM float_sweeps s JOIN balance_reservations r ON r.payout_id=s.payout_id")
        .fetch_one(&mut **tx).await?;
    if let Some(resolved) = resolved {
        for role in [AccountRole::Collection, AccountRole::Disbursement] {
            let latest: Option<i64> = sqlx::query_scalar("SELECT created_at FROM balance_queries WHERE role=? AND applied_at IS NOT NULL ORDER BY sequence DESC LIMIT 1")
                .bind(role.name()).fetch_optional(&mut **tx).await?;
            if latest.is_none_or(|time| time <= resolved) {
                return Err(Error::LiquidityUnavailable.into());
            }
        }
    }
    let last: Option<i64> = sqlx::query_scalar("SELECT max(created_at) FROM float_sweeps")
        .fetch_one(&mut **tx)
        .await?;
    if last.is_some_and(|t| now.0 < (t as u64).saturating_add(u64::from(config.cooldown_seconds))) {
        return Err(RepositoryError::Conflict);
    }
    let amounts: Vec<String> =
        sqlx::query_scalar("SELECT amount FROM float_sweeps WHERE created_at>=?")
            .bind(time(Timestamp(now.0.saturating_sub(86400)))?)
            .fetch_all(&mut **tx)
            .await?;
    let mut daily = gross;
    for amount in amounts {
        daily = daily.checked_add(minor(&amount)?)?;
    }
    if daily.value() > config.max_daily.value() {
        return Err(Error::InsufficientFloat.into());
    }
    sqlx::query("INSERT INTO float_sweeps(payout_id,query_id,created_at,amount) VALUES(?,?,?,?)")
        .bind(payout.dispatch().operation_id().as_str())
        .bind(claim.snapshot.as_str())
        .bind(time(now)?)
        .bind(gross.value().to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

impl Repository {
    pub(crate) async fn recheck_dispatch_float(
        &self,
        operation: &Id,
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.write().await?;
        let payout = super::dispatch::load(&mut tx, operation).await?;
        if payout.state() != OutboundState::Dispatching {
            return Err(Error::InvalidTransition.into());
        }
        if let Some(quote) = payout.dispatch().quote() {
            quote.validate_dispatch(now)?;
        }
        let sweep: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM float_sweeps WHERE payout_id=?)")
                .bind(operation.as_str())
                .fetch_one(&mut *tx)
                .await?;
        let role = if sweep {
            AccountRole::Collection
        } else {
            AccountRole::Disbursement
        };
        if sweep {
            let utility = status(&mut tx, AccountRole::Disbursement, now).await?;
            if utility.available.is_none() || !utility.alert {
                return Err(Error::LiquidityUnavailable.into());
            }
        }
        let s = status(&mut tx, role, now).await?;
        if s.available.is_none() {
            return Err(Error::LiquidityUnavailable.into());
        }
        let required = s
            .pending_liability
            .checked_add(s.quote_reservations)?
            .checked_add(policy(&mut tx).await?.minimum)?;
        if s.balance.ok_or(Error::LiquidityUnavailable)?.value() < required.value() {
            return Err(Error::InsufficientFloat.into());
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn liquidity_alerts(&self) -> Result<Vec<(AccountRole, String, bool, Timestamp)>> {
        let rows =
            sqlx::query("SELECT role,reason,active,updated_at FROM liquidity_alerts ORDER BY role")
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    match row.try_get::<String, _>("role")?.as_str() {
                        "collection" => AccountRole::Collection,
                        "disbursement" => AccountRole::Disbursement,
                        _ => return Err(RepositoryError::Encoding),
                    },
                    row.try_get("reason")?,
                    row.try_get("active")?,
                    Timestamp(
                        row.try_get::<i64, _>("updated_at")?
                            .try_into()
                            .map_err(|_| Error::Overflow)?,
                    ),
                ))
            })
            .collect()
    }
}

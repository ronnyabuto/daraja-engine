use super::dispatch::{load, persist};
use super::*;
use crate::{
    collection::StatusResult,
    crypto::Secret,
    outbound::PayoutTarget,
    payout::PayoutResult,
    transport::{ClientError, Outcome},
};

impl Repository {
    pub(crate) async fn claim_outbound_job(
        &self,
        now: Timestamp,
        automated: u32,
        hard_ttl: u32,
    ) -> Result<Option<JobClaim>> {
        let until = Timestamp(now.0.checked_add(30).ok_or(Error::Overflow)?);
        let row = sqlx::query("UPDATE durable_jobs SET lease_until=?,generation=generation+1 WHERE id=(SELECT j.id FROM durable_jobs j JOIN outbound_payouts p ON p.id=j.subject WHERE j.kind='RECONCILE_PAYOUT' AND j.completed_at IS NULL AND (j.due_at<=? OR (p.state NOT IN ('ManualReconciliationRequired','Paid','Failed') AND (p.manual_after<=? OR json_extract(p.data,'$.started_at')+?<=? OR json_extract(p.data,'$.started_at')+?<=?))) AND (j.lease_until IS NULL OR j.lease_until<=?) ORDER BY j.due_at,j.id LIMIT 1) RETURNING id,subject,generation,poll_slot,retry_count")
            .bind(time(until)?).bind(time(now)?).bind(time(now)?).bind(automated).bind(time(now)?).bind(hard_ttl).bind(time(now)?).bind(time(now)?).fetch_optional(&self.pool).await?;
        row.map(|r| {
            Ok(JobClaim {
                id: Id::new(r.try_get::<String, _>("id")?)?,
                kind: "RECONCILE_PAYOUT".into(),
                subject: Id::new(r.try_get::<String, _>("subject")?)?,
                generation: r.try_get("generation")?,
                lease_until: until,
                poll_slot: r.try_get("poll_slot")?,
                retry_count: r.try_get("retry_count")?,
            })
        })
        .transpose()
    }
    pub(crate) async fn payout_target(&self, id: &Id) -> Result<PayoutTarget> {
        let mut tx = self.pool.begin().await?;
        let payout = load(&mut tx, id).await?;
        let admission =
            sqlx::query("SELECT endpoint,command FROM payout_dispatches WHERE payout_id=?")
                .bind(id.as_str())
                .fetch_optional(&mut *tx)
                .await?;
        let endpoint = admission
            .as_ref()
            .map(|r| r.try_get("endpoint"))
            .transpose()?;
        let command = admission
            .as_ref()
            .map(|r| r.try_get("command"))
            .transpose()?;
        let initial: Option<String> = sqlx::query_scalar(
            "SELECT outcome FROM payout_dispatch_observations WHERE payout_id=?",
        )
        .bind(id.as_str())
        .fetch_optional(&mut *tx)
        .await?;
        let attempts: Vec<Option<String>> =
            sqlx::query_scalar("SELECT outcome FROM payout_reconcile_attempts WHERE payout_id=?")
                .bind(id.as_str())
                .fetch_all(&mut *tx)
                .await?;
        let duplicate: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM payout_dispatch_observations WHERE payout_id=? AND disposition='Duplicate') OR EXISTS(SELECT 1 FROM payout_reconcile_attempts WHERE payout_id=? AND kind='PROBE')")
            .bind(id.as_str()).bind(id.as_str()).fetch_one(&mut *tx).await?;
        let callbacks: Vec<String> = sqlx::query_scalar("SELECT i.body FROM callback_inbox i JOIN payout_callback_links l ON l.observation_id=i.id WHERE l.payout_id=? AND i.endpoint IN ('/callbacks/b2c/{secret}','/callbacks/b2b/{secret}') ORDER BY i.id")
            .bind(id.as_str()).fetch_all(&mut *tx).await?;
        let status_rows = sqlx::query("SELECT i.body,q.lookup_id FROM callback_inbox i JOIN outbound_status_queries q ON json_extract(i.body,'$.Result.OriginatorConversationID')=q.originator AND json_extract(i.body,'$.Result.ConversationID')=q.conversation WHERE q.payout_id=? AND i.endpoint='/callbacks/status/result/{secret}' ORDER BY i.id")
            .bind(id.as_str()).fetch_all(&mut *tx).await?;
        let mut ids = std::collections::BTreeSet::new();
        for text in initial.iter().chain(attempts.iter().flatten()) {
            let value: serde_json::Value = serde_json::from_str(text)?;
            if let Some(s) = value["transaction_id"].as_str() {
                ids.insert(Id::new(s)?);
            }
        }
        let mut contradictory = false;
        let mut codes = std::collections::BTreeSet::new();
        for body in &callbacks {
            match PayoutResult::parse(body) {
                Ok(r) => {
                    codes.insert(r.observation.result_code.as_str().to_owned());
                    if r.amount
                        .is_some_and(|a| a != payout.dispatch().payload().amount)
                    {
                        contradictory = true;
                    }
                    if let Some(receipt) = r.receipt {
                        ids.insert(Id::new(receipt.as_str())?);
                    }
                }
                Err(_) => contradictory = true,
            }
        }
        let mut status_callbacks = Vec::new();
        for row in status_rows {
            let body: String = row.try_get("body")?;
            let expected: String = row.try_get("lookup_id")?;
            match StatusResult::parse(&body) {
                Ok(r) => {
                    codes.insert(r.result_code.as_str().to_owned());
                    if r.observed_amount
                        .is_some_and(|a| a != payout.dispatch().payload().amount)
                        || r.receipt.as_ref().is_some_and(|r| r.as_str() != expected)
                        || r.transaction_id
                            .as_ref()
                            .is_some_and(|r| r.as_str() != expected)
                    {
                        contradictory = true;
                    }
                }
                Err(_) => contradictory = true,
            }
            status_callbacks.push(Secret::new(body));
        }
        contradictory |= ids.len() > 1 || codes.len() > 1;
        Ok(PayoutTarget {
            payout,
            endpoint,
            command,
            transaction_id: if contradictory {
                None
            } else {
                ids.into_iter().next()
            },
            callbacks: callbacks.into_iter().map(Secret::new).collect(),
            status_callbacks,
            probe_consumed: duplicate,
            contradictory,
        })
    }
    pub(crate) async fn begin_outbound_attempt(
        &self,
        claim: &JobClaim,
        probe: bool,
        transaction: Option<&Id>,
        gate: Option<&Id>,
        now: Timestamp,
    ) -> Result<Id> {
        let mut tx = self.write().await?;
        check_lease(&mut tx, claim, now).await?;
        let p = load(&mut tx, &claim.subject).await?;
        if matches!(
            p.state(),
            OutboundState::Paid
                | OutboundState::Failed
                | OutboundState::ManualReconciliationRequired
        ) {
            return Err(Error::InvalidTransition.into());
        }
        let query = Id::new(format!(
            "out-status:{}:{}",
            claim.subject.as_str(),
            claim.generation
        ))?;
        sqlx::query("INSERT INTO payout_reconcile_attempts(payout_id,generation,kind,evidence_gate,created_at) VALUES(?,?,?,?,?)")
            .bind(claim.subject.as_str()).bind(claim.generation).bind(if probe {"PROBE"} else {"STATUS"})
            .bind(gate.map(Id::as_str)).bind(time(now)?).execute(&mut *tx).await?;
        if !probe {
            // Read the candidate in the worker snapshot; no proxy/caller ID is substituted for a transaction ID.
            let transaction = transaction.ok_or(Error::EvidenceMismatch)?;
            sqlx::query("INSERT INTO outbound_status_queries(id,payout_id,lookup_kind,lookup_id,created_at) VALUES(?,?,'TransactionID',?,?)")
                .bind(query.as_str()).bind(claim.subject.as_str()).bind(transaction.as_str()).bind(time(now)?).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE durable_jobs SET poll_slot=poll_slot+1 WHERE id=?")
            .bind(claim.id.as_str())
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            &claim.subject,
            if probe {
                "PROBE_PREPARED"
            } else {
                "STATUS_PREPARED"
            },
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(query)
    }
    pub(crate) async fn check_outbound_probe(
        &self,
        claim: &JobClaim,
        gate: &Id,
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        check_lease(&mut tx, claim, now).await?;
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM payout_reconcile_attempts a JOIN outbound_payouts p ON p.id=a.payout_id WHERE a.payout_id=? AND a.generation=? AND a.kind='PROBE' AND a.evidence_gate=? AND a.outcome IS NULL AND p.state IN ('Dispatching','UnknownInFlight','Pending'))")
            .bind(claim.subject.as_str()).bind(claim.generation).bind(gate.as_str()).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(Error::InvalidTransition.into());
        }
        Ok(())
    }
    pub(crate) async fn record_outbound_attempt(
        &self,
        claim: &JobClaim,
        result: Option<std::result::Result<Outcome, ClientError>>,
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.write().await?;
        check_lease(&mut tx, claim, now).await?;
        if let Some(result) = result {
            let duplicate = result
                .as_ref()
                .err()
                .and_then(|e| e.outcome.provider_code.as_ref())
                .is_some_and(|c| c.as_str() == "500.002.1001");
            let ack = result.is_ok();
            let outcome = match result {
                Ok(o) => o,
                Err(e) => *e.outcome,
            };
            let mut p = load(&mut tx, &claim.subject).await?;
            if ack {
                if let (Some(o), Some(c)) = (
                    &outcome.originator_conversation_id,
                    &outcome.conversation_id,
                ) {
                    p.record_acknowledgment(o.clone(), c.clone())?;
                }
            } else if !matches!(p.state(), OutboundState::Paid | OutboundState::Failed) {
                if duplicate {
                    p.duplicate_observed()?;
                } else {
                    p.ambiguous(Ambiguity::TransportError)?;
                }
            }
            sqlx::query(
                "UPDATE payout_reconcile_attempts SET outcome=? WHERE payout_id=? AND generation=?",
            )
            .bind(encode(&outcome)?)
            .bind(claim.subject.as_str())
            .bind(claim.generation)
            .execute(&mut *tx)
            .await?;
            persist(&mut tx, &claim.subject, &p).await?;
            super::dispatch::link_callbacks(&mut tx, now, Some(&claim.subject), None).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn finish_outbound(
        &self,
        claim: &JobClaim,
        evidence: Option<PayoutAttestation>,
        manual: Option<&str>,
        retry: Option<Timestamp>,
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.write().await?;
        check_lease(&mut tx, claim, now).await?;
        let mut p = load(&mut tx, &claim.subject).await?;
        if !matches!(p.state(), OutboundState::Paid | OutboundState::Failed) {
            if let Some(e) = evidence {
                // Payout::resolve checks all frozen dispatch bindings and requires a receipt for Paid.
                p.resolve(e)?;
                sqlx::query("UPDATE balance_reservations SET released_at=?,evidence=? WHERE payout_id=? AND released_at IS NULL")
                    .bind(time(now)?).bind(encode(&p.final_evidence())?).bind(claim.subject.as_str()).execute(&mut *tx).await?;
                audit(&mut tx, &claim.subject, "VERIFIED_PAYOUT_RESOLUTION", now).await?;
            } else if let Some(reason) = manual
                && p.state() != OutboundState::ManualReconciliationRequired
            {
                p.require_manual()?;
                sqlx::query("INSERT INTO payout_alerts(payout_id,reason,created_at) VALUES(?,?,?) ON CONFLICT DO NOTHING")
                    .bind(claim.subject.as_str()).bind(reason).bind(time(now)?).execute(&mut *tx).await?;
                audit(
                    &mut tx,
                    &claim.subject,
                    "MANUAL_RECONCILIATION_REQUIRED",
                    now,
                )
                .await?;
            }
            persist(&mut tx, &claim.subject, &p).await?;
        }
        let terminal = matches!(p.state(), OutboundState::Paid | OutboundState::Failed);
        let next = if terminal {
            None
        } else if manual.is_some() {
            Some(Timestamp(now.0.checked_add(60).ok_or(Error::Overflow)?))
        } else {
            retry
        };
        sqlx::query("UPDATE durable_jobs SET due_at=coalesce(?,due_at),completed_at=?,lease_until=NULL WHERE id=? AND generation=?")
            .bind(next.map(time).transpose()?).bind(if terminal {Some(time(now)?)} else {None})
            .bind(claim.id.as_str()).bind(claim.generation).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn check_outbound_query(&self, query: &Id, transaction: &Id) -> Result<()> {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM outbound_status_queries WHERE id=? AND lookup_kind='TransactionID' AND lookup_id=?)")
            .bind(query.as_str()).bind(transaction.as_str()).fetch_one(&self.pool).await?;
        if !exists {
            return Err(Error::EvidenceMismatch.into());
        }
        Ok(())
    }
    pub(crate) async fn outbound_status_ack(
        &self,
        id: &Id,
        originator: &Id,
        conversation: &Id,
    ) -> Result<()> {
        changed(sqlx::query("UPDATE outbound_status_queries SET originator=?,conversation=? WHERE id=? AND (originator IS NULL OR originator=?) AND (conversation IS NULL OR conversation=?)")
            .bind(originator.as_str()).bind(conversation.as_str()).bind(id.as_str()).bind(originator.as_str()).bind(conversation.as_str()).execute(&self.pool).await?.rows_affected())
    }
    /// Durable local alerts. Delivery and authenticated operator resolution are stage 15.
    pub async fn payout_alerts(&self) -> Result<Vec<(Id, String, Timestamp)>> {
        sqlx::query(
            "SELECT payout_id,reason,created_at FROM payout_alerts ORDER BY created_at,payout_id",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|r| {
            Ok((
                Id::new(r.try_get::<String, _>("payout_id")?)?,
                r.try_get("reason")?,
                Timestamp(
                    r.try_get::<i64, _>("created_at")?
                        .try_into()
                        .map_err(|_| Error::Overflow)?,
                ),
            ))
        })
        .collect()
    }
}

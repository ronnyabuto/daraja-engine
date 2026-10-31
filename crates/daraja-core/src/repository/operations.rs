use super::*;
use crate::{
    inbound::EvidenceSource,
    operations::{AlertClaim, PayoutView, QueueEntry, QueueKind, ResolvePayout},
    outbound::FinalOutcome,
};
use sha2::{Digest, Sha256};

fn view(p: &Payout, version: i64) -> PayoutView {
    PayoutView {
        state: p.state(),
        cdk_state: p.state().cdk_state(),
        version,
        amount: p.dispatch().payload().amount,
        payload_hash: p.dispatch().hash(),
        has_final_evidence: p.final_evidence().is_some(),
    }
}

impl Repository {
    pub(crate) async fn operator_review_deposit_hold(
        &self,
        actor: &Id,
        request: &crate::operations::ReviewDepositHold,
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.write().await?;
        let data = encode(request)?;
        let previous: Option<String> = sqlx::query_scalar(
            "SELECT request FROM operator_hold_reviews WHERE actor=? AND command_id=?",
        )
        .bind(actor.as_str())
        .bind(request.command_id.as_str())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(previous) = previous {
            if previous != data {
                return Err(Error::IdempotencyConflict.into());
            }
            return Ok(());
        }
        changed(
            sqlx::query("DELETE FROM deposit_blocks WHERE quote_id=? AND reason=?")
                .bind(request.quote_id.as_str())
                .bind(request.expected_reason.as_str())
                .execute(&mut *tx)
                .await?
                .rows_affected(),
        )?;
        sqlx::query("INSERT INTO operator_hold_reviews VALUES(?,?,?,?,?)")
            .bind(actor.as_str())
            .bind(request.command_id.as_str())
            .bind(request.quote_id.as_str())
            .bind(&data)
            .bind(time(now)?)
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            &request.quote_id,
            &format!("HOLD_REVIEWED:{}:{}", actor.as_str(), data),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn operator_queue(
        &self,
        kind: QueueKind,
        after: i64,
        limit: u32,
    ) -> Result<Vec<QueueEntry>> {
        if after < 0 || limit == 0 || limit > 100 {
            return Err(Error::InvalidAmount.into());
        }
        let sql = match kind {
            QueueKind::PendingPayouts => {
                "SELECT rowid AS cursor,id AS subject,data FROM outbound_payouts WHERE rowid>? AND state NOT IN ('Paid','Failed') ORDER BY rowid LIMIT ?"
            }
            QueueKind::ExceptionDeposits => {
                "SELECT r.rowid AS cursor,r.receipt AS subject,r.data FROM mpesa_receipts_ledger r JOIN unreconciled_deposits u ON u.receipt=r.receipt WHERE r.rowid>? AND u.resolved_at IS NULL ORDER BY r.rowid LIMIT ?"
            }
        };
        sqlx::query(sql)
            .bind(after)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|row| {
                let data: String = row.try_get("data")?;
                let (state, amount) = match kind {
                    QueueKind::PendingPayouts => {
                        let p: Payout = decode(&data)?;
                        (state(&p.state())?, p.dispatch().payload().amount)
                    }
                    QueueKind::ExceptionDeposits => {
                        let r: ConfirmedReceipt = decode(&data)?;
                        (state(&r.state)?, r.deposit.evidence().amount)
                    }
                };
                let subject: String = row.try_get("subject")?;
                Ok(QueueEntry {
                    cursor: row.try_get("cursor")?,
                    subject_hash: crate::identity::PayloadHash::from_bytes(
                        Sha256::digest(subject.as_bytes()).into(),
                    ),
                    state,
                    amount,
                })
            })
            .collect()
    }

    pub(crate) async fn operator_claim_alert(
        &self,
        actor: &Id,
        now: Timestamp,
    ) -> Result<Option<AlertClaim>> {
        let mut tx = self.write().await?;
        let until = Timestamp(now.0.checked_add(30).ok_or(Error::Overflow)?);
        let row = sqlx::query("UPDATE operator_alert_deliveries SET lease_actor=?,lease_until=?,generation=generation+1,attempts=attempts+1 WHERE sequence=(SELECT sequence FROM operator_alert_deliveries WHERE delivered_at IS NULL AND due_at<=? AND (lease_until IS NULL OR lease_until<=?) ORDER BY sequence LIMIT 1) RETURNING sequence,generation,kind,subject,reason,active,attempts")
            .bind(actor.as_str()).bind(time(until)?).bind(time(now)?).bind(time(now)?).fetch_optional(&mut *tx).await?;
        let claim = row
            .map(|r| -> Result<AlertClaim> {
                Ok(AlertClaim {
                    sequence: r.try_get("sequence")?,
                    generation: r.try_get("generation")?,
                    kind: r.try_get("kind")?,
                    subject: Id::new(r.try_get::<String, _>("subject")?)?,
                    reason: r.try_get("reason")?,
                    active: r.try_get("active")?,
                    attempts: r.try_get("attempts")?,
                    lease_until: until,
                })
            })
            .transpose()?;
        if let Some(claim) = &claim {
            audit(
                &mut tx,
                &Id::new(format!("operator-alert:{}", claim.sequence))?,
                &format!("ALERT_CLAIMED:{}:{}", actor.as_str(), claim.generation),
                now,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(claim)
    }

    pub(crate) async fn operator_finish_alert(
        &self,
        actor: &Id,
        claim: &AlertClaim,
        evidence: Option<&Id>,
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.write().await?;
        let row = sqlx::query("SELECT lease_actor,generation,lease_until,attempts,delivered_at,delivery_evidence FROM operator_alert_deliveries WHERE sequence=?")
            .bind(claim.sequence).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        if row.try_get::<Option<String>, _>("lease_actor")?.as_deref() != Some(actor.as_str())
            || row.try_get::<i64, _>("generation")? != claim.generation
        {
            return Err(RepositoryError::LeaseLost);
        }
        if row.try_get::<Option<i64>, _>("delivered_at")?.is_some() {
            if evidence.is_none()
                || row
                    .try_get::<Option<String>, _>("delivery_evidence")?
                    .as_deref()
                    != evidence.map(Id::as_str)
            {
                return Err(Error::IdempotencyConflict.into());
            }
            return Ok(());
        }
        let current_time = time(now)?;
        if row
            .try_get::<Option<i64>, _>("lease_until")?
            .is_none_or(|until| until <= current_time || current_time < until.saturating_sub(30))
        {
            return Err(RepositoryError::LeaseLost);
        }
        let attempts: i64 = row.try_get("attempts")?;
        let delay = 5u64.saturating_mul(1u64 << attempts.clamp(0, 6)).min(300);
        let due = Timestamp(now.0.checked_add(delay).ok_or(Error::Overflow)?);
        sqlx::query("UPDATE operator_alert_deliveries SET delivered_at=?,delivery_evidence=?,due_at=?,lease_until=NULL WHERE sequence=?")
            .bind(if evidence.is_some() {Some(time(now)?)} else {None}).bind(evidence.map(Id::as_str)).bind(time(due)?).bind(claim.sequence).execute(&mut *tx).await?;
        audit(
            &mut tx,
            &Id::new(format!("operator-alert:{}", claim.sequence))?,
            &format!(
                "ALERT_{}:{}:{}",
                if evidence.is_some() {
                    "DELIVERED"
                } else {
                    "RETRY"
                },
                actor.as_str(),
                evidence.map(Id::as_str).unwrap_or("SINK_UNAVAILABLE")
            ),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn operator_resolve(
        &self,
        actor: &Id,
        request: &ResolvePayout,
        now: Timestamp,
    ) -> Result<(PayoutView, bool)> {
        let hash: [u8; 32] = Sha256::digest(encode(request)?.as_bytes()).into();
        let mut tx = self.write().await?;
        if let Some(row) = sqlx::query("SELECT request_hash,result,result_version,outcome FROM operator_commands WHERE actor=? AND command_id=?")
            .bind(actor.as_str()).bind(request.command_id.as_str()).fetch_optional(&mut *tx).await?
        {
            if row.try_get::<Vec<u8>, _>("request_hash")? != hash {
                return Err(Error::IdempotencyConflict.into());
            }
            let p: Payout = decode(row.try_get("result")?)?;
            return Ok((view(&p, row.try_get("result_version")?), row.try_get::<String,_>("outcome")? == "CONTRADICTORY"));
        }
        let row = sqlx::query("SELECT data,version FROM outbound_payouts WHERE id=?")
            .bind(request.payout_id.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
        let mut p: Payout = decode(row.try_get("data")?)?;
        let mut version: i64 = row.try_get("version")?;
        if version != request.expected_version {
            return Err(RepositoryError::Conflict);
        }
        if now < p.started_at() {
            return Err(Error::InvalidTransition.into());
        }
        let source_ref = match &request.evidence.source {
            EvidenceSource::ReviewedStatement(id)
            | EvidenceSource::AuthenticatedProviderRecord(id) => id,
        };
        if source_ref != &request.evidence_ref
            || request.evidence.operation_id != request.payout_id
            || request.evidence.payload_hash != p.dispatch().hash()
            || request.evidence.amount != p.dispatch().payload().amount
            || request.evidence.destination != p.dispatch().payload().destination
            || matches!(&request.evidence.outcome, FinalOutcome::DefinitivelyNotExecuted { evidence_record } if evidence_record != &request.evidence_ref)
        {
            return Err(Error::EvidenceMismatch.into());
        }
        let terminal = matches!(p.state(), OutboundState::Paid | OutboundState::Failed);
        let contradictory = terminal
            && p.final_evidence()
                .is_some_and(|e| e.outcome != request.evidence.outcome);
        if !terminal {
            if p.state() != OutboundState::ManualReconciliationRequired {
                return Err(Error::InvalidTransition.into());
            }
            p.resolve(request.evidence.clone())?;
            changed(sqlx::query("UPDATE outbound_payouts SET state=?,data=?,version=version+1 WHERE id=? AND version=?")
                .bind(state(&p.state())?).bind(encode(&p)?).bind(request.payout_id.as_str()).bind(version)
                .execute(&mut *tx).await?.rows_affected())?;
            version = version.checked_add(1).ok_or(Error::Overflow)?;
            changed(sqlx::query("UPDATE balance_reservations SET released_at=?,evidence=? WHERE payout_id=? AND released_at IS NULL")
                .bind(time(now)?).bind(encode(&p.final_evidence())?).bind(request.payout_id.as_str())
                .execute(&mut *tx).await?.rows_affected())?;
            // Fence an in-flight worker before it can commit an older reconciliation snapshot.
            sqlx::query("UPDATE durable_jobs SET completed_at=?,lease_until=NULL,generation=generation+1 WHERE kind='RECONCILE_PAYOUT' AND subject=?")
                .bind(time(now)?).bind(request.payout_id.as_str()).execute(&mut *tx).await?;
        }
        let outcome = if contradictory {
            "CONTRADICTORY"
        } else {
            "APPLIED"
        };
        sqlx::query("INSERT INTO operator_commands(actor,command_id,subject,reason,evidence_ref,expected_version,request_hash,request,outcome,result,result_version,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(actor.as_str()).bind(request.command_id.as_str()).bind(request.payout_id.as_str())
            .bind(request.reason.as_str()).bind(request.evidence_ref.as_str()).bind(request.expected_version)
            .bind(hash.as_slice()).bind(encode(request)?).bind(outcome).bind(encode(&p)?).bind(version).bind(time(now)?)
            .execute(&mut *tx).await?;
        audit(
            &mut tx,
            &request.payout_id,
            &format!(
                "OPERATOR_RESOLUTION:{outcome}:{}:{}",
                actor.as_str(),
                request.command_id.as_str()
            ),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok((view(&p, version), contradictory))
    }
}

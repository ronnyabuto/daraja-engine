use super::*;
use crate::{
    inbound::EvidenceSource,
    operations::{PayoutView, ResolvePayout},
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

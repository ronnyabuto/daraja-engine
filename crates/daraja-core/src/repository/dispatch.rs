use super::*;
use crate::transport::Outcome;

impl Repository {
    pub(crate) async fn payout_replay(
        &self,
        key: &RequestKey,
        payload: &RequestPayload,
    ) -> Result<Option<Payout>> {
        let row = sqlx::query("SELECT b.payload_hash,p.data FROM request_bindings b JOIN outbound_payouts p ON p.id=b.operation_id WHERE b.caller=? AND b.operation=? AND b.request_key=?")
            .bind(key.caller.as_str()).bind(key.operation.as_str()).bind(key.key.as_str())
            .fetch_optional(&self.pool).await?;
        match row {
            Some(row) => {
                if row.try_get::<Vec<u8>, _>("payload_hash")? != payload.hash().as_bytes() {
                    return Err(Error::IdempotencyConflict.into());
                }
                let payout: Payout = decode(row.try_get("data")?)?;
                if payout.dispatch().payload() != payload {
                    return Err(Error::IdempotencyConflict.into());
                }
                Ok(Some(payout))
            }
            None => Ok(None),
        }
    }

    pub(crate) async fn record_dispatch_outcome(
        &self,
        id: &Id,
        acknowledgment: Option<(Id, Id)>,
        disposition: &str,
        outcome: &Outcome,
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.write().await?;
        let mut payout = load(&mut tx, id).await?;
        if let Some((originator, conversation)) = acknowledgment {
            payout.record_acknowledgment(originator, conversation)?;
        } else if !matches!(payout.state(), OutboundState::Paid | OutboundState::Failed) {
            if disposition == "Duplicate" {
                payout.duplicate_observed()?;
            } else {
                payout.ambiguous(Ambiguity::TransportError)?;
            }
        }
        sqlx::query("INSERT INTO payout_dispatch_observations(payout_id,disposition,outcome,observed_at) VALUES(?,?,?,?) ON CONFLICT(payout_id) DO NOTHING")
            .bind(id.as_str()).bind(disposition).bind(encode(outcome)?).bind(time(now)?)
            .execute(&mut *tx).await?;
        persist(&mut tx, id, &payout).await?;
        link_callbacks(&mut tx, now, Some(id), None).await?;
        audit(&mut tx, id, "DISPATCH_OBSERVED", now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Startup recovery only marks potentially sent records; it never grants send permission.
    pub async fn recover_dispatching_payouts(&self, now: Timestamp) -> Result<u64> {
        let mut tx = self.write().await?;
        let rows = sqlx::query("SELECT id,data FROM outbound_payouts WHERE state='Dispatching'")
            .fetch_all(&mut *tx)
            .await?;
        for row in &rows {
            let id = Id::new(row.try_get::<String, _>("id")?)?;
            let mut payout: Payout = decode(row.try_get("data")?)?;
            payout.ambiguous(Ambiguity::WorkerRecovery)?;
            persist(&mut tx, &id, &payout).await?;
            audit(&mut tx, &id, "DISPATCH_RECOVERY_UNKNOWN", now).await?;
        }
        tx.commit().await?;
        Ok(rows.len() as u64)
    }

    /// Untrusted correlated observations only. They cannot authorize final financial states.
    pub async fn payout_callback_ids(&self, id: &Id) -> Result<Vec<Id>> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT observation_id FROM payout_callback_links WHERE payout_id=? ORDER BY observation_id")
            .bind(id.as_str()).fetch_all(&self.pool).await?;
        rows.into_iter().map(|s| Ok(Id::new(s)?)).collect()
    }
}

async fn load(tx: &mut Transaction<'_, Sqlite>, id: &Id) -> Result<Payout> {
    let data: String = sqlx::query_scalar("SELECT data FROM outbound_payouts WHERE id=?")
        .bind(id.as_str())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(Error::NotFound)?;
    decode(&data)
}
async fn persist(tx: &mut Transaction<'_, Sqlite>, id: &Id, p: &Payout) -> Result<()> {
    sqlx::query("UPDATE outbound_payouts SET state=?,provider_originator=?,provider_conversation=?,data=?,version=version+1 WHERE id=?")
        .bind(state(&p.state())?).bind(p.correlation().provider_originator.as_ref().map(Id::as_str))
        .bind(p.correlation().provider_conversation.as_ref().map(Id::as_str)).bind(encode(p)?)
        .bind(id.as_str()).execute(&mut **tx).await?;
    Ok(())
}

pub(super) async fn link_callbacks(
    tx: &mut Transaction<'_, Sqlite>,
    now: Timestamp,
    payout_id: Option<&Id>,
    observation_id: Option<&str>,
) -> Result<()> {
    // Proxy IDs may be unknown until acknowledgment. Caller-ID observations can be
    // linked earlier, but never supply or replace the trusted acknowledgment binding.
    sqlx::query("INSERT INTO payout_callback_links(observation_id,payout_id) SELECT i.id,p.id FROM callback_inbox i JOIN outbound_payouts p ON ((json_extract(i.body,'$.Result.OriginatorConversationID')=json_extract(p.data,'$.correlation.originator_conversation') AND (p.provider_conversation IS NULL OR json_extract(i.body,'$.Result.ConversationID')=p.provider_conversation)) OR (json_extract(i.body,'$.Result.OriginatorConversationID')=p.provider_originator AND json_extract(i.body,'$.Result.ConversationID')=p.provider_conversation)) JOIN payout_dispatches d ON d.payout_id=p.id WHERE (? IS NULL OR p.id=?) AND (? IS NULL OR i.id=?) AND ((d.command='BusinessPayment' AND i.endpoint IN ('/callbacks/b2c/{secret}','/callbacks/b2c/timeout/{secret}')) OR (d.command IN ('BusinessBuyGoods','BusinessPayBill') AND i.endpoint IN ('/callbacks/b2b/{secret}','/callbacks/b2b/timeout/{secret}'))) ON CONFLICT(observation_id) DO NOTHING")
        .bind(payout_id.map(Id::as_str)).bind(payout_id.map(Id::as_str))
        .bind(observation_id).bind(observation_id).execute(&mut **tx).await?;
    let ids: Vec<String> = sqlx::query_scalar("SELECT DISTINCT p.id FROM outbound_payouts p JOIN payout_callback_links l ON l.payout_id=p.id JOIN callback_inbox i ON i.id=l.observation_id WHERE (? IS NULL OR p.id=?) AND (? IS NULL OR i.id=?) AND p.state IN ('Dispatching','Pending') AND i.endpoint IN ('/callbacks/b2c/timeout/{secret}','/callbacks/b2b/timeout/{secret}')")
        .bind(payout_id.map(Id::as_str)).bind(payout_id.map(Id::as_str))
        .bind(observation_id).bind(observation_id).fetch_all(&mut **tx).await?;
    for id in ids {
        let id = Id::new(id)?;
        let mut p = load(tx, &id).await?;
        p.ambiguous(Ambiguity::QueueTimeout)?;
        persist(tx, &id, &p).await?;
        audit(tx, &id, "PAYOUT_QUEUE_TIMEOUT", now).await?;
    }
    Ok(())
}

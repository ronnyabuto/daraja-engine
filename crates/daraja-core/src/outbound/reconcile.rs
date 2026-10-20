use std::{future::Future, time::Duration};

use super::{OutboundState, Payout, PayoutAttestation};
use crate::{
    Error, Timestamp,
    collection::{
        StatusAcknowledgment, StatusIdentifierType, StatusLookup, StatusPersistence,
        StatusPrepared, StatusRequest, TransactionStatusClient,
    },
    crypto::{Clock, Secret},
    identity::Id,
    payout::PayoutClient,
    repository::{JobClaim, Repository, RepositoryError},
    transport::{ClientError, ErrorKind, HttpTransport, Outcome},
};

type Result<T> = std::result::Result<T, RepositoryError>;

#[derive(Clone, Debug)]
pub struct ReconcileConfig {
    pub poll_interval_seconds: u32,
    pub automated_poll_seconds: u32,
    pub max_pending_ttl_seconds: u32,
    pub max_attempts: u32,
    /// Independently reviewed guarantees, scoped to the exact endpoint and command.
    /// Empty by default. Documentation alone does not establish this guarantee.
    pub probe_evidence: Vec<ProbeEvidence>,
}
impl Default for ReconcileConfig {
    fn default() -> Self {
        Self {
            poll_interval_seconds: 60,
            automated_poll_seconds: 7200,
            max_pending_ttl_seconds: 43200,
            max_attempts: 120,
            probe_evidence: Vec::new(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct ProbeEvidence {
    pub endpoint: reqwest::Url,
    pub command: String,
    pub evidence_record: Id,
    pub verified_until: Timestamp,
    /// Maximum payout age covered by the verified duplicate retention guarantee.
    pub max_operation_age_seconds: u32,
}
impl ReconcileConfig {
    fn validate(&self) -> Result<()> {
        if self.poll_interval_seconds == 0
            || self.automated_poll_seconds == 0
            || self.max_pending_ttl_seconds < self.automated_poll_seconds
            || self.max_attempts == 0
        {
            return Err(Error::InvalidTransition.into());
        }
        Ok(())
    }
    fn permit<'a>(&'a self, target: &PayoutTarget, now: Timestamp) -> Option<&'a ProbeEvidence> {
        self.probe_evidence.iter().find(|e| {
            Some(e.endpoint.as_str()) == target.endpoint.as_deref()
                && Some(e.command.as_str()) == target.command.as_deref()
                && matches!(
                    e.command.as_str(),
                    "BusinessPayment" | "BusinessBuyGoods" | "BusinessPayBill"
                )
                && now < e.verified_until
                && now.0.saturating_sub(target.payout.started_at().0)
                    < u64::from(e.max_operation_age_seconds)
        })
    }
}

/// Callback bodies are untrusted hints; the verifier must use independent provider evidence.
#[derive(Debug)]
pub struct PayoutTarget {
    pub payout: Payout,
    pub endpoint: Option<String>,
    pub command: Option<String>,
    pub transaction_id: Option<Id>,
    pub callbacks: Vec<Secret>,
    pub status_callbacks: Vec<Secret>,
    pub probe_consumed: bool,
    pub contradictory: bool,
}

pub trait PayoutVerifier: Send + Sync {
    /// Establish exact operation, destination, value and finality independently of callbacks.
    /// Definitive non-execution requires documented evidence or audited operator authority.
    fn evidence(
        &self,
        target: &PayoutTarget,
        now: Timestamp,
    ) -> impl Future<Output = Option<PayoutAttestation>> + Send;
}
pub struct UnavailablePayoutVerifier;
impl PayoutVerifier for UnavailablePayoutVerifier {
    async fn evidence(&self, _: &PayoutTarget, _: Timestamp) -> Option<PayoutAttestation> {
        None
    }
}

pub trait OutboundQueries: Send + Sync {
    fn probe(
        &self,
        target: &PayoutTarget,
        claim: &JobClaim,
        gate: &ProbeEvidence,
        send_before: Timestamp,
    ) -> impl Future<Output = std::result::Result<Outcome, ClientError>> + Send;
    fn status(&self, query: &Id, transaction: &Id) -> impl Future<Output = ()> + Send;
}

pub struct OutboundReconciler<Q, V, C> {
    pub repository: Repository,
    pub queries: Q,
    pub verifier: V,
    pub clock: C,
    pub config: ReconcileConfig,
}
impl<Q: OutboundQueries, V: PayoutVerifier, C: Clock> OutboundReconciler<Q, V, C> {
    fn now(&self) -> Result<Timestamp> {
        Ok(Timestamp(
            self.clock
                .now()
                .timestamp()
                .try_into()
                .map_err(|_| Error::Overflow)?,
        ))
    }
    pub async fn run_once(&self) -> Result<bool> {
        self.config.validate()?;
        let now = self.now()?;
        let Some(claim) = self
            .repository
            .claim_outbound_job(
                now,
                self.config.automated_poll_seconds,
                self.config.max_pending_ttl_seconds,
            )
            .await?
        else {
            return Ok(false);
        };
        let target = self.repository.payout_target(&claim.subject).await?;
        if matches!(
            target.payout.state(),
            OutboundState::Paid | OutboundState::Failed
        ) {
            self.repository.finish_job(&claim, now, None).await?;
            return Ok(true);
        }
        // Manual work consumes independently available late evidence without issuing probes or status queries.
        let evidence =
            tokio::time::timeout(Duration::from_secs(5), self.verifier.evidence(&target, now))
                .await
                .ok()
                .flatten();
        if let Some(e) = evidence {
            self.repository
                .finish_outbound(&claim, Some(e), None, None, self.now()?)
                .await?;
            return Ok(true);
        }
        let now = self.now()?;
        let age = now.0.saturating_sub(target.payout.started_at().0);
        let reason = if target.payout.state() == OutboundState::ManualReconciliationRequired {
            Some("MANUAL_REVIEW")
        } else if now >= target.payout.manual_after()
            || age >= u64::from(self.config.max_pending_ttl_seconds)
        {
            Some("MAX_PENDING_TTL")
        } else if age >= u64::from(self.config.automated_poll_seconds)
            || claim.poll_slot >= i64::from(self.config.max_attempts)
        {
            Some("POLLING_EXHAUSTED")
        } else if target.contradictory {
            Some("CONTRADICTORY_OBSERVATIONS")
        } else {
            None
        };
        if let Some(reason) = reason {
            self.repository
                .finish_outbound(&claim, None, Some(reason), None, self.now()?)
                .await?;
            return Ok(true);
        }
        let permit = self.config.permit(&target, now);
        let probe = target.payout.correlation().provider_conversation.is_none()
            && target.transaction_id.is_none()
            && !target.probe_consumed
            && permit.is_some();
        if !probe && target.transaction_id.is_none() {
            self.repository
                .finish_outbound(
                    &claim,
                    None,
                    Some("NO_SAFE_QUERY_OR_PROBE"),
                    None,
                    self.now()?,
                )
                .await?;
            return Ok(true);
        }
        let query = self
            .repository
            .begin_outbound_attempt(
                &claim,
                probe,
                target.transaction_id.as_ref(),
                permit.map(|p| &p.evidence_record),
                self.now()?,
            )
            .await?;
        let deadline = target
            .payout
            .started_at()
            .0
            .checked_add(u64::from(self.config.max_pending_ttl_seconds))
            .ok_or(Error::Overflow)?
            .min(target.payout.manual_after().0)
            .min(
                target
                    .payout
                    .started_at()
                    .0
                    .checked_add(u64::from(self.config.automated_poll_seconds))
                    .ok_or(Error::Overflow)?,
            );
        let io = async {
            if probe {
                Some(
                    self.queries
                        .probe(
                            &target,
                            &claim,
                            permit.expect("probe evidence"),
                            Timestamp(deadline),
                        )
                        .await,
                )
            } else {
                self.queries
                    .status(
                        &query,
                        target.transaction_id.as_ref().expect("queryable ID"),
                    )
                    .await;
                None
            }
        };
        let result = tokio::time::timeout(Duration::from_secs(15), io)
            .await
            .ok()
            .flatten();
        let now = self.now()?;
        self.repository
            .record_outbound_attempt(&claim, result, now)
            .await?;
        let next = Timestamp(
            now.0
                .checked_add(u64::from(self.config.poll_interval_seconds))
                .ok_or(Error::Overflow)?
                .min(deadline),
        );
        self.repository
            .finish_outbound(&claim, None, None, Some(next), now)
            .await?;
        Ok(true)
    }
}

pub struct OutboundProviderQueries<T, S, C> {
    pub payout: PayoutClient<T>,
    pub status: TransactionStatusClient<S, C>,
    pub repository: Repository,
    pub clock: C,
    pub status_config: crate::reconcile::InboundStatusConfig,
}
impl<T: HttpTransport, S: HttpTransport, C: Clock + Send + Sync> OutboundQueries
    for OutboundProviderQueries<T, S, C>
{
    async fn probe(
        &self,
        target: &PayoutTarget,
        claim: &JobClaim,
        gate: &ProbeEvidence,
        send_before: Timestamp,
    ) -> std::result::Result<Outcome, ClientError> {
        self.payout
            .probe_frozen(
                &target.payout,
                target
                    .endpoint
                    .as_deref()
                    .ok_or_else(|| ClientError::new(ErrorKind::CapabilityDisabled))?,
                target
                    .command
                    .as_deref()
                    .ok_or_else(|| ClientError::new(ErrorKind::CapabilityDisabled))?,
                async {
                    let now = Timestamp(
                        self.clock
                            .now()
                            .timestamp()
                            .try_into()
                            .map_err(|_| ClientError::new(ErrorKind::Input))?,
                    );
                    if now >= send_before
                        || now >= gate.verified_until
                        || now.0.saturating_sub(target.payout.started_at().0)
                            >= u64::from(gate.max_operation_age_seconds)
                        || Some(gate.endpoint.as_str()) != target.endpoint.as_deref()
                        || Some(gate.command.as_str()) != target.command.as_deref()
                    {
                        return Err(ClientError::new(ErrorKind::CapabilityDisabled));
                    }
                    self.repository
                        .check_outbound_probe(claim, &gate.evidence_record, now)
                        .await
                        .map_err(|_| ClientError::new(ErrorKind::Persistence))
                },
            )
            .await
    }
    async fn status(&self, query: &Id, transaction: &Id) {
        let c = &self.status_config;
        let _ = self
            .status
            .request(
                &StatusRequest {
                    local_query_id: query.clone(),
                    lookup: StatusLookup::Receipt(transaction.clone()),
                    initiator: c.initiator.clone(),
                    security_credential: c.security_credential.clone(),
                    party_a: c.shortcode.clone(),
                    identifier_type: StatusIdentifierType::Shortcode,
                    result_url: c.result_url.clone(),
                    timeout_url: c.timeout_url.clone(),
                    remarks: Secret::new("outbound reconciliation"),
                    occasion: Secret::new("outbound"),
                },
                &OutboundStatusHooks(&self.repository),
            )
            .await;
    }
}
struct OutboundStatusHooks<'a>(&'a Repository);
impl StatusPersistence for OutboundStatusHooks<'_> {
    async fn prepare(&self, record: StatusPrepared<'_>) -> crate::transport::Result<()> {
        let StatusLookup::Receipt(id) = record.lookup else {
            return Err(ClientError::new(ErrorKind::CapabilityDisabled));
        };
        self.0
            .check_outbound_query(record.local_query_id, id)
            .await
            .map_err(|_| ClientError::new(ErrorKind::Persistence))
    }
    async fn acknowledge(
        &self,
        id: &Id,
        ack: &StatusAcknowledgment,
    ) -> crate::transport::Result<()> {
        self.0
            .outbound_status_ack(id, &ack.originator_conversation_id, &ack.conversation_id)
            .await
            .map_err(|_| ClientError::new(ErrorKind::Persistence))
    }
}

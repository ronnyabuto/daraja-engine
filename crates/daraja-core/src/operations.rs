//! Deployment-owned operator access; never mount this boundary on an application API.
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{
    Timestamp,
    amount::Amount,
    crypto::Secret,
    identity::{Id, PayloadHash},
    outbound::{CdkPaymentState, OutboundState, PayoutAttestation},
    repository::{Repository, RepositoryError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatorRole {
    Observer,
    Reconciler,
}

/// Construct only from deployment configuration, separately from client credentials.
pub struct OperatorCredential {
    actor: Id,
    digest: [u8; 32],
    role: OperatorRole,
}
impl OperatorCredential {
    pub fn new(actor: Id, token: &Secret, role: OperatorRole) -> Result<Self, OperatorError> {
        if token.expose().len() < 32 || token.expose().len() > 512 {
            return Err(OperatorError::InvalidCredential);
        }
        Ok(Self {
            actor,
            digest: Sha256::digest(token.expose().as_bytes()).into(),
            role,
        })
    }
}

#[derive(Debug)]
pub enum OperatorError {
    InvalidCredential,
    Unauthorized,
    Forbidden,
    ContradictoryEvidence,
    Repository(RepositoryError),
    Dispatch(crate::outbound::DispatchError),
}
impl From<RepositoryError> for OperatorError {
    fn from(value: RepositoryError) -> Self {
        Self::Repository(value)
    }
}
impl std::fmt::Display for OperatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for OperatorError {}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ResolvePayout {
    pub command_id: Id,
    pub payout_id: Id,
    pub expected_version: i64,
    /// Opaque case reference, not free text containing customer data.
    pub reason: Id,
    pub evidence_ref: Id,
    /// Independently reviewed transaction/portal record, never callback-derived.
    pub evidence: PayoutAttestation,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ReviewDepositHold {
    pub command_id: Id,
    pub quote_id: Id,
    pub expected_reason: Id,
    pub reason: Id,
    pub evidence_ref: Id,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RetryVerification {
    pub command_id: Id,
    pub job_id: Id,
    pub expected_generation: i64,
    pub reason: Id,
    pub evidence_ref: Id,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RequoteHeldFx {
    pub command_id: Id,
    pub original_quote: Id,
    pub expected_version: i64,
    pub replacement_quote: Id,
    pub account_ref: Id,
    pub owner: Id,
    pub reason: Id,
    pub evidence_ref: Id,
}

/// Contains no wire body, destination, callback URL, receipt or bearer material.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct PayoutView {
    pub state: OutboundState,
    pub cdk_state: CdkPaymentState,
    pub version: i64,
    pub amount: Amount,
    pub payload_hash: PayloadHash,
    pub has_final_evidence: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum QueueKind {
    PendingPayouts,
    ExceptionDeposits,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct QueueEntry {
    pub cursor: i64,
    pub subject_hash: PayloadHash,
    pub state: String,
    pub amount: Amount,
}

/// Sink delivery is at least once. Deduplicate on sequence; acknowledge only
/// after the external sink durably accepts the alert.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AlertClaim {
    pub sequence: i64,
    pub generation: i64,
    pub kind: String,
    pub subject: Id,
    pub reason: String,
    pub active: bool,
    pub attempts: i64,
    pub lease_until: Timestamp,
}

pub struct OperatorService {
    repository: Repository,
    credentials: Vec<OperatorCredential>,
}
impl OperatorService {
    pub async fn requote_held_fx<O: crate::fx::FxOracle>(
        &self,
        token: &Secret,
        request: &RequoteHeldFx,
        oracle: &O,
        now: Timestamp,
    ) -> Result<crate::inbound::SettlementEvent, OperatorError> {
        let actor = self.authenticate(token, true)?;
        if let Some(event) = self
            .repository
            .operator_requote_replay(actor, request)
            .await?
        {
            return Ok(event);
        }
        let fx = oracle.quote(now).await.map_err(|_| {
            OperatorError::Repository(RepositoryError::Domain(crate::Error::FxExpired))
        })?;
        Ok(self
            .repository
            .operator_requote_fx(actor, request, fx, now)
            .await?)
    }

    pub async fn request_refund(
        &self,
        token: &Secret,
        id: &Id,
        mut authorization: crate::repository::RefundAuthorization,
        tariff: crate::amount::TariffSnapshot,
        now: Timestamp,
    ) -> Result<crate::repository::RefundRequest, OperatorError> {
        let actor = self.authenticate(token, true)?;
        if authorization.receipts.len() > 100 {
            return Err(OperatorError::Repository(RepositoryError::Domain(
                crate::Error::InvalidAmount,
            )));
        }
        authorization.operator = actor.clone();
        Ok(self
            .repository
            .request_refund(id, authorization, tariff, now)
            .await?)
    }

    pub async fn execute_refund<
        T: crate::transport::HttpTransport,
        C: crate::crypto::Clock + Send + Sync,
    >(
        &self,
        token: &Secret,
        refund_id: &Id,
        reason: &Id,
        quote: &crate::valuation::PaymentQuote,
        payments: &crate::outbound::OutboundPayments<T, C>,
        now: Timestamp,
    ) -> Result<crate::outbound::Payout, OperatorError> {
        let actor = self.authenticate(token, true)?;
        self.repository
            .operator_bind_refund(actor, reason, refund_id, quote, now)
            .await?;
        payments
            .execute_refund(quote.id().clone())
            .await
            .map_err(OperatorError::Dispatch)
    }

    pub async fn retry_verification(
        &self,
        token: &Secret,
        request: &RetryVerification,
        now: Timestamp,
    ) -> Result<(), OperatorError> {
        let actor = self.authenticate(token, true)?;
        Ok(self
            .repository
            .operator_retry_verification(actor, request, now)
            .await?)
    }

    pub async fn review_deposit_hold(
        &self,
        token: &Secret,
        request: &ReviewDepositHold,
        now: Timestamp,
    ) -> Result<(), OperatorError> {
        let actor = self.authenticate(token, true)?;
        Ok(self
            .repository
            .operator_review_deposit_hold(actor, request, now)
            .await?)
    }

    pub async fn queue(
        &self,
        token: &Secret,
        kind: QueueKind,
        after: i64,
        limit: u32,
    ) -> Result<Vec<QueueEntry>, OperatorError> {
        self.authenticate(token, false)?;
        Ok(self.repository.operator_queue(kind, after, limit).await?)
    }

    pub async fn claim_alert(
        &self,
        token: &Secret,
        now: Timestamp,
    ) -> Result<Option<AlertClaim>, OperatorError> {
        let actor = self.authenticate(token, true)?;
        Ok(self.repository.operator_claim_alert(actor, now).await?)
    }

    /// None retries with persisted bounded exponential backoff. Some evidence
    /// acknowledges durable external delivery, never a financial resolution.
    pub async fn finish_alert(
        &self,
        token: &Secret,
        claim: &AlertClaim,
        evidence: Option<&Id>,
        now: Timestamp,
    ) -> Result<(), OperatorError> {
        let actor = self.authenticate(token, true)?;
        Ok(self
            .repository
            .operator_finish_alert(actor, claim, evidence, now)
            .await?)
    }

    pub fn new(
        repository: Repository,
        credentials: Vec<OperatorCredential>,
    ) -> Result<Self, OperatorError> {
        if credentials.len() > 64 {
            return Err(OperatorError::InvalidCredential);
        }
        for (i, credential) in credentials.iter().enumerate() {
            if credentials[..i]
                .iter()
                .any(|c| c.digest == credential.digest || c.actor == credential.actor)
            {
                return Err(OperatorError::InvalidCredential);
            }
        }
        Ok(Self {
            repository,
            credentials,
        })
    }

    fn authenticate(&self, token: &Secret, write: bool) -> Result<&Id, OperatorError> {
        if token.expose().len() > 512 {
            return Err(OperatorError::Unauthorized);
        }
        let digest: [u8; 32] = Sha256::digest(token.expose().as_bytes()).into();
        let mut matched = None;
        for credential in &self.credentials {
            if bool::from(credential.digest.ct_eq(&digest)) {
                matched = Some(credential);
            }
        }
        let credential = matched.ok_or(OperatorError::Unauthorized)?;
        if write && credential.role != OperatorRole::Reconciler {
            return Err(OperatorError::Forbidden);
        }
        Ok(&credential.actor)
    }

    pub async fn payout(&self, token: &Secret, id: &Id) -> Result<PayoutView, OperatorError> {
        self.authenticate(token, false)?;
        let (p, version) = self.repository.payout(id).await?;
        Ok(PayoutView {
            state: p.state(),
            cdk_state: p.state().cdk_state(),
            version,
            amount: p.dispatch().payload().amount,
            payload_hash: p.dispatch().hash(),
            has_final_evidence: p.final_evidence().is_some(),
        })
    }

    pub async fn resolve_payout(
        &self,
        token: &Secret,
        request: &ResolvePayout,
        now: Timestamp,
    ) -> Result<PayoutView, OperatorError> {
        let actor = self.authenticate(token, true)?;
        let (view, contradictory) = self
            .repository
            .operator_resolve(actor, request, now)
            .await?;
        if contradictory {
            return Err(OperatorError::ContradictoryEvidence);
        }
        Ok(view)
    }
}

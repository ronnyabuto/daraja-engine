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

pub struct OperatorService {
    repository: Repository,
    credentials: Vec<OperatorCredential>,
}
impl OperatorService {
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

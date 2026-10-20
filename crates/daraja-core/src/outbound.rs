use serde::Serialize;

use crate::{
    Error, Timestamp,
    amount::{Amount, TariffSnapshot},
    identity::{Correlation, Id, PayloadHash, ReceiptId, RequestPayload},
    inbound::EvidenceSource,
};

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutboundState {
    Dispatching,
    UnknownInFlight,
    Pending,
    Paid,
    Failed,
    ManualReconciliationRequired,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CdkPaymentState {
    Pending,
    Paid,
    Failed,
}

impl OutboundState {
    pub fn cdk_state(self) -> CdkPaymentState {
        match self {
            Self::Paid => CdkPaymentState::Paid,
            Self::Failed => CdkPaymentState::Failed,
            _ => CdkPaymentState::Pending,
        }
    }
}

/// Frozen before dispatch. Persistence must commit this record before provider I/O.
#[derive(Serialize, Clone, PartialEq, Eq)]
pub struct DispatchRecord {
    operation_id: Id,
    payload: RequestPayload,
    hash: PayloadHash,
    wire_body: Vec<u8>,
    tariff: TariffSnapshot,
    reserved: Amount,
    quote: Option<crate::valuation::PaymentQuote>,
}

impl std::fmt::Debug for DispatchRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DispatchRecord([redacted])")
    }
}

impl DispatchRecord {
    pub fn new(
        operation_id: Id,
        payload: RequestPayload,
        wire_body: Vec<u8>,
        tariff: TariffSnapshot,
        reserved: Amount,
    ) -> Result<Self, Error> {
        payload.amount.require_kes()?;
        if payload.amount.value() == 0 || wire_body.is_empty() {
            return Err(Error::InvalidAmount);
        }
        if payload.tariff_version != tariff.version {
            return Err(Error::EvidenceMismatch);
        }
        if payload.amount.checked_add(tariff.fee)? != reserved {
            return Err(Error::AmountMismatch);
        }
        Ok(Self {
            operation_id,
            hash: payload.hash(),
            payload,
            wire_body,
            tariff,
            reserved,
            quote: None,
        })
    }

    pub fn with_quote(
        mut self,
        quote: crate::valuation::PaymentQuote,
        now: Timestamp,
    ) -> Result<Self, Error> {
        quote.validate_dispatch(now)?;
        if self.payload.quote_id != *quote.id()
            || self.payload.destination != *quote.destination()
            || self.payload.amount != quote.principal()
            || self.tariff != *quote.tariff()
            || self.reserved != quote.principal().checked_add(quote.tariff().fee)?
        {
            return Err(Error::EvidenceMismatch);
        }
        self.quote = Some(quote);
        Ok(self)
    }
    pub fn quote(&self) -> Option<&crate::valuation::PaymentQuote> {
        self.quote.as_ref()
    }

    pub fn operation_id(&self) -> &Id {
        &self.operation_id
    }
    pub fn payload(&self) -> &RequestPayload {
        &self.payload
    }
    pub fn hash(&self) -> PayloadHash {
        self.hash
    }
    pub fn wire_body(&self) -> &[u8] {
        &self.wire_body
    }
    pub fn tariff(&self) -> &TariffSnapshot {
        &self.tariff
    }
    pub fn reserved(&self) -> Amount {
        self.reserved
    }
}

/// Final evidence must refer to this exact dispatch. Correlation alone is not provenance.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct PayoutAttestation {
    pub source: EvidenceSource,
    pub operation_id: Id,
    pub payload_hash: PayloadHash,
    pub amount: Amount,
    pub destination: Id,
    pub outcome: FinalOutcome,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub enum FinalOutcome {
    Paid {
        receipt: ReceiptId,
    },
    /// An independently verified final nonexecution, not a timeout, absence or duplicate error.
    DefinitivelyNotExecuted {
        evidence_record: Id,
    },
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ambiguity {
    TransportError,
    Http500,
    Http503,
    Timeout,
    QueueTimeout,
    WorkerRecovery,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Payout {
    dispatch: DispatchRecord,
    correlation: Correlation,
    state: OutboundState,
    started_at: Timestamp,
    next_check_at: Timestamp,
    manual_after: Timestamp,
    final_evidence: Option<PayoutAttestation>,
}

impl Payout {
    pub fn dispatching(
        dispatch: DispatchRecord,
        caller_request: Id,
        started_at: Timestamp,
        next_check_at: Timestamp,
        manual_after: Timestamp,
    ) -> Result<Self, Error> {
        if let Some(quote) = dispatch.quote() {
            quote.validate_dispatch(started_at)?;
        }
        if next_check_at < started_at || manual_after < next_check_at {
            return Err(Error::InvalidTransition);
        }
        Ok(Self {
            correlation: Correlation {
                caller_request,
                originator_conversation: dispatch.operation_id.clone(),
                provider_originator: None,
                provider_conversation: None,
            },
            dispatch,
            state: OutboundState::Dispatching,
            started_at,
            next_check_at,
            manual_after,
            final_evidence: None,
        })
    }

    pub fn state(&self) -> OutboundState {
        self.state
    }
    pub fn dispatch(&self) -> &DispatchRecord {
        &self.dispatch
    }
    pub fn correlation(&self) -> &Correlation {
        &self.correlation
    }
    pub fn started_at(&self) -> Timestamp {
        self.started_at
    }
    pub fn next_check_at(&self) -> Timestamp {
        self.next_check_at
    }
    pub fn manual_after(&self) -> Timestamp {
        self.manual_after
    }
    pub fn final_evidence(&self) -> Option<&PayoutAttestation> {
        self.final_evidence.as_ref()
    }

    fn nonterminal(&self) -> Result<(), Error> {
        if matches!(self.state, OutboundState::Paid | OutboundState::Failed) {
            return Err(Error::InvalidTransition);
        }
        Ok(())
    }

    pub fn acknowledge(
        &mut self,
        provider_originator: Id,
        provider_conversation: Id,
    ) -> Result<(), Error> {
        self.nonterminal()?;
        self.record_acknowledgment(provider_originator, provider_conversation)
    }

    pub(crate) fn record_acknowledgment(
        &mut self,
        provider_originator: Id,
        provider_conversation: Id,
    ) -> Result<(), Error> {
        self.correlation
            .acknowledge(provider_originator, provider_conversation)?;
        if matches!(self.state, OutboundState::Paid | OutboundState::Failed) {
            return Ok(());
        }
        if self.state != OutboundState::ManualReconciliationRequired {
            self.state = OutboundState::Pending;
        }
        Ok(())
    }

    pub fn ambiguous(&mut self, reason: Ambiguity) -> Result<(), Error> {
        self.nonterminal()?;
        if self.state == OutboundState::Dispatching
            || (reason == Ambiguity::QueueTimeout && self.state == OutboundState::Pending)
        {
            self.state = OutboundState::UnknownInFlight;
        }
        Ok(())
    }

    /// Duplicate rejection is an observation, never final settlement or permission to retry.
    pub fn duplicate_observed(&mut self) -> Result<(), Error> {
        self.nonterminal()?;
        if self.state != OutboundState::ManualReconciliationRequired {
            self.state = OutboundState::Pending;
        }
        Ok(())
    }

    pub fn escalate(&mut self, now: Timestamp) -> Result<(), Error> {
        self.nonterminal()?;
        if now < self.manual_after {
            return Err(Error::InvalidTransition);
        }
        self.state = OutboundState::ManualReconciliationRequired;
        Ok(())
    }

    /// Caller must obtain independently trusted final evidence; this validates binding only.
    pub fn resolve(&mut self, evidence: PayoutAttestation) -> Result<(), Error> {
        self.nonterminal()?;
        if evidence.operation_id != self.dispatch.operation_id
            || evidence.payload_hash != self.dispatch.hash
            || evidence.amount != self.dispatch.payload.amount
            || evidence.destination != self.dispatch.payload.destination
        {
            return Err(Error::EvidenceMismatch);
        }
        self.state = match evidence.outcome {
            FinalOutcome::Paid { .. } => OutboundState::Paid,
            FinalOutcome::DefinitivelyNotExecuted { .. } => OutboundState::Failed,
        };
        self.final_evidence = Some(evidence);
        Ok(())
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "OutboundState")]
enum StoredOutboundState {
    Dispatching,
    UnknownInFlight,
    Pending,
    Paid,
    Failed,
    ManualReconciliationRequired,
}
impl crate::storage::Stored for OutboundState {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredOutboundState::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "CdkPaymentState")]
enum StoredCdkPaymentState {
    Pending,
    Paid,
    Failed,
}
impl crate::storage::Stored for CdkPaymentState {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredCdkPaymentState::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "DispatchRecord")]
struct StoredDispatchRecord {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    operation_id: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    payload: RequestPayload,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    hash: PayloadHash,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    wire_body: Vec<u8>,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    tariff: TariffSnapshot,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    reserved: Amount,
    #[serde(default, deserialize_with = "crate::storage::deserialize")]
    quote: Option<crate::valuation::PaymentQuote>,
}
impl crate::storage::Stored for DispatchRecord {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredDispatchRecord::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "PayoutAttestation")]
struct StoredPayoutAttestation {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    source: EvidenceSource,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    operation_id: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    payload_hash: PayloadHash,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    amount: Amount,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    destination: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    outcome: FinalOutcome,
}
impl crate::storage::Stored for PayoutAttestation {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredPayoutAttestation::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "FinalOutcome")]
enum StoredFinalOutcome {
    Paid {
        #[serde(deserialize_with = "crate::storage::deserialize")]
        receipt: ReceiptId,
    },
    DefinitivelyNotExecuted {
        #[serde(deserialize_with = "crate::storage::deserialize")]
        evidence_record: Id,
    },
}
impl crate::storage::Stored for FinalOutcome {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredFinalOutcome::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "Ambiguity")]
enum StoredAmbiguity {
    TransportError,
    Http500,
    Http503,
    Timeout,
    QueueTimeout,
    WorkerRecovery,
}
impl crate::storage::Stored for Ambiguity {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredAmbiguity::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "Payout")]
struct StoredPayout {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    dispatch: DispatchRecord,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    correlation: Correlation,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    state: OutboundState,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    started_at: Timestamp,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    next_check_at: Timestamp,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    manual_after: Timestamp,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    final_evidence: Option<PayoutAttestation>,
}
impl crate::storage::Stored for Payout {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredPayout::deserialize(d)
    }
}

mod dispatch;
pub use dispatch::{DispatchConfig, DispatchError, MakePayment, OutboundPayments};

use serde::Serialize;

use std::collections::BTreeMap;

use crate::{
    Error, Timestamp,
    amount::{Amount, FxSnapshot, TariffSnapshot, Unit},
    identity::{Id, ReceiptId},
};

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum InboundState {
    Issued,
    StkSent,
    Verifying,
    UnderpaidHeld,
    OverpaidHeld,
    SettlementHeld,
    Paid,
    RefundHeld,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct QuoteTerms {
    pub expected: Amount,
    pub credit: Amount,
    pub quote_expires_at: Timestamp,
    pub fx: Option<FxSnapshot>,
    pub tariff: TariffSnapshot,
    pub hold_until: Timestamp,
}

impl QuoteTerms {
    pub fn validate(&self) -> Result<(), Error> {
        self.expected.require_kes()?;
        if self.expected.value() == 0 || self.credit.value() == 0 {
            return Err(Error::InvalidAmount);
        }
        let net = self.expected.checked_sub(self.tariff.fee)?;
        let credit = match (self.credit.unit(), &self.fx) {
            (Unit::KesMinor, None) => net,
            (Unit::Sat, Some(fx)) => fx.credit(net)?,
            _ => return Err(Error::CurrencyMismatch),
        };
        if credit != self.credit {
            return Err(Error::AmountMismatch);
        }
        Ok(())
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptState {
    Dispatching,
    Sent,
    Unknown,
    Completed,
    Failed,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct StkAttempt {
    id: Id,
    started_at: Timestamp,
    next_poll_at: Timestamp,
    resend_after: Timestamp,
    state: AttemptState,
    caller_request: Id,
    merchant_request: Option<Id>,
    checkout_request: Option<Id>,
}

impl StkAttempt {
    pub fn new(id: Id, caller_request: Id, started_at: Timestamp) -> Result<Self, Error> {
        let next = Timestamp(started_at.0.checked_add(35).ok_or(Error::Overflow)?);
        Ok(Self {
            caller_request,
            merchant_request: None,
            id,
            started_at,
            next_poll_at: next,
            resend_after: next,
            state: AttemptState::Dispatching,
            checkout_request: None,
        })
    }

    pub fn id(&self) -> &Id {
        &self.id
    }
    pub fn started_at(&self) -> Timestamp {
        self.started_at
    }
    pub fn next_poll_at(&self) -> Timestamp {
        self.next_poll_at
    }
    pub fn resend_after(&self) -> Timestamp {
        self.resend_after
    }
    pub fn state(&self) -> AttemptState {
        self.state
    }
    pub fn caller_request(&self) -> &Id {
        &self.caller_request
    }
    pub fn merchant_request(&self) -> Option<&Id> {
        self.merchant_request.as_ref()
    }
    pub fn checkout_request(&self) -> Option<&Id> {
        self.checkout_request.as_ref()
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct InboundQuote {
    id: Id,
    account_ref: Id,
    shortcode: Id,
    terms: QuoteTerms,
    state: InboundState,
    attempts: Vec<StkAttempt>,
}

impl InboundQuote {
    pub fn new(id: Id, account_ref: Id, shortcode: Id, terms: QuoteTerms) -> Result<Self, Error> {
        terms.validate()?;
        if account_ref.as_str().len() != 8
            || !account_ref
                .as_str()
                .bytes()
                .all(|b| b.is_ascii_alphanumeric())
        {
            return Err(Error::InvalidIdentity);
        }
        Ok(Self {
            id,
            account_ref,
            shortcode,
            terms,
            state: InboundState::Issued,
            attempts: vec![],
        })
    }

    pub fn id(&self) -> &Id {
        &self.id
    }
    pub fn account_ref(&self) -> &Id {
        &self.account_ref
    }
    pub fn shortcode(&self) -> &Id {
        &self.shortcode
    }

    pub fn state(&self) -> InboundState {
        self.state
    }
    pub fn terms(&self) -> &QuoteTerms {
        &self.terms
    }
    pub(crate) fn accepts_receipt_time(
        &self,
        at: Timestamp,
        kes_grace: u64,
    ) -> Result<bool, Error> {
        Ok(at < self.terms.quote_expires_at
            || (self.terms.credit.unit() == Unit::KesMinor
                && at.0
                    < self
                        .terms
                        .quote_expires_at
                        .0
                        .checked_add(kes_grace)
                        .ok_or(Error::Overflow)?))
    }

    pub fn attempts(&self) -> &[StkAttempt] {
        &self.attempts
    }

    pub fn start_stk(&mut self, attempt: StkAttempt) -> Result<(), Error> {
        if !matches!(self.state, InboundState::Issued | InboundState::StkSent) {
            return Err(Error::InvalidTransition);
        }
        if attempt.started_at >= self.terms.quote_expires_at {
            return Err(Error::QuoteExpired);
        }
        if self
            .terms
            .fx
            .as_ref()
            .is_some_and(|fx| attempt.started_at >= fx.expires_at)
        {
            return Err(Error::FxExpired);
        }
        if let Some(last) = self.attempts.last()
            && attempt.started_at < last.resend_after
        {
            return Err(Error::SettlementHeld);
        }
        if self.attempts.iter().any(|a| a.id == attempt.id) {
            return Err(Error::DuplicateIdentity);
        }
        self.attempts.push(attempt);
        Ok(())
    }

    pub fn acknowledge_stk(
        &mut self,
        attempt: &Id,
        merchant: Id,
        checkout: Id,
    ) -> Result<(), Error> {
        if self.state == InboundState::Paid {
            return Err(Error::InvalidTransition);
        }
        let stored = self
            .attempts
            .iter()
            .find(|a| &a.id == attempt)
            .ok_or(Error::NotFound)?;
        if !matches!(
            stored.state,
            AttemptState::Dispatching | AttemptState::Unknown | AttemptState::Sent
        ) {
            return Err(Error::InvalidTransition);
        }
        self.record_stk_ack(attempt, merchant, checkout)
    }

    /// Provider correlations remain useful when the manual rail settled before the acknowledgment.
    pub(crate) fn record_stk_ack(
        &mut self,
        attempt: &Id,
        merchant: Id,
        checkout: Id,
    ) -> Result<(), Error> {
        if self
            .attempts
            .iter()
            .any(|a| a.id != *attempt && a.checkout_request.as_ref() == Some(&checkout))
        {
            return Err(Error::DuplicateIdentity);
        }
        let stored = self
            .attempts
            .iter_mut()
            .find(|a| &a.id == attempt)
            .ok_or(Error::NotFound)?;
        if stored
            .checkout_request
            .as_ref()
            .is_some_and(|id| id != &checkout)
        {
            return Err(Error::EvidenceMismatch);
        }
        if stored
            .merchant_request
            .as_ref()
            .is_some_and(|id| id != &merchant)
        {
            return Err(Error::EvidenceMismatch);
        }
        stored.merchant_request = Some(merchant);
        stored.checkout_request = Some(checkout);
        if !matches!(stored.state, AttemptState::Completed | AttemptState::Failed) {
            stored.state = AttemptState::Sent;
        }
        if matches!(self.state, InboundState::Issued | InboundState::StkSent) {
            self.state = InboundState::StkSent;
        }
        Ok(())
    }

    /// A transport failure changes only the attempt; the C2B rail remains usable.
    pub fn stk_unknown(&mut self, attempt: &Id) -> Result<(), Error> {
        if self.state == InboundState::Paid {
            return Err(Error::InvalidTransition);
        }
        let attempt = self
            .attempts
            .iter_mut()
            .find(|a| &a.id == attempt)
            .ok_or(Error::NotFound)?;
        if !matches!(
            attempt.state,
            AttemptState::Dispatching | AttemptState::Sent | AttemptState::Unknown
        ) {
            return Err(Error::InvalidTransition);
        }
        attempt.state = AttemptState::Unknown;
        Ok(())
    }

    /// Attempt-level observation; success here never verifies or credits a deposit.
    pub fn finish_stk(&mut self, attempt: &Id, succeeded: bool) -> Result<(), Error> {
        if self.state == InboundState::Paid {
            return Err(Error::InvalidTransition);
        }
        let attempt = self
            .attempts
            .iter_mut()
            .find(|a| &a.id == attempt)
            .ok_or(Error::NotFound)?;
        if matches!(
            attempt.state,
            AttemptState::Completed | AttemptState::Failed
        ) {
            return Err(Error::InvalidTransition);
        }
        attempt.state = if succeeded {
            AttemptState::Completed
        } else {
            AttemptState::Failed
        };
        Ok(())
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub enum DepositRail {
    Stk { checkout_request: Id },
    C2b { account_ref: Id },
}

/// An observation is not a ledger reservation, even if it claims success.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ReceiptObservation {
    pub observation_id: Id,
    pub claimed_receipt: ReceiptId,
    pub claimed_quote: Option<Id>,
    pub claimed_amount: Amount,
    pub rail: DepositRail,
    pub observed_at: Timestamp,
}

/// A reference to independently reviewed evidence, not a callback/query correlation ID.
/// Construction is an explicit trusted-verifier boundary, not provider verification.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub enum EvidenceSource {
    AuthenticatedProviderRecord(Id),
    ReviewedStatement(Id),
}

/// A stable pseudonymous identity from the trusted verifier, not a raw MSISDN.
#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct VerifiedPayer {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    pub subject: Id,
    pub identity_verified: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct DepositAttestation {
    /// Independently established payer identity; never copied from callback MSISDN.
    pub payer: Option<VerifiedPayer>,
    pub source: EvidenceSource,
    pub receipt: ReceiptId,
    pub quote_id: Option<Id>,
    pub amount: Amount,
    pub shortcode: Id,
    pub rail: DepositRail,
    pub received_at: Timestamp,
    pub verified_at: Timestamp,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct VerifiedDeposit(DepositAttestation);

impl VerifiedDeposit {
    /// Only a trusted verifier may attest final receipt/amount/destination/time/binding.
    /// No network verifier is supplied in stage 02; live SETTLE remains disabled.
    pub fn attest(evidence: DepositAttestation) -> Result<Self, Error> {
        evidence.amount.require_kes()?;
        if evidence.amount.value() == 0 || evidence.received_at > evidence.verified_at {
            return Err(Error::InvalidAmount);
        }
        Ok(Self(evidence))
    }

    pub fn evidence(&self) -> &DepositAttestation {
        &self.0
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepositState {
    UnderpaidHeld,
    OverpaidHeld,
    OrphanUnclaimed,
    LateHeld,
    FxExpiredHeld,
    SettlementHeld,
    Verified,
    Settled,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub enum Allocation {
    Quote(Id),
    Exception,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ConfirmedReceipt {
    pub deposit: VerifiedDeposit,
    pub allocation: Allocation,
    pub state: DepositState,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct SettlementEvent {
    /// One settlement event per quote; stable replay identity.
    pub quote_id: Id,
    pub credit: Amount,
    pub receipts: Vec<ReceiptId>,
    pub settled_at: Timestamp,
}

/// Pure accounting aggregate, NOT a durable repository. Stage 03 must persist
/// receipt allocations, quote mutation and this outbox in one transaction/CAS.
#[derive(Default)]
pub struct Accounting {
    pub(crate) blocked: bool,
    pub(crate) quotes: BTreeMap<Id, InboundQuote>,
    observations: Vec<ReceiptObservation>,
    pub(crate) receipts: BTreeMap<ReceiptId, ConfirmedReceipt>,
    events: BTreeMap<Id, SettlementEvent>,
}

impl Accounting {
    pub fn insert_quote(&mut self, quote: InboundQuote) -> Result<(), Error> {
        if self.quotes.contains_key(&quote.id)
            || self
                .quotes
                .values()
                .any(|q| q.account_ref == quote.account_ref)
        {
            return Err(Error::DuplicateIdentity);
        }
        self.quotes.insert(quote.id.clone(), quote);
        Ok(())
    }

    pub fn quote(&self, id: &Id) -> Option<&InboundQuote> {
        self.quotes.get(id)
    }
    pub fn receipt(&self, id: &ReceiptId) -> Option<&ConfirmedReceipt> {
        self.receipts.get(id)
    }
    pub fn events(&self) -> &BTreeMap<Id, SettlementEvent> {
        &self.events
    }
    pub fn observations(&self) -> &[ReceiptObservation] {
        &self.observations
    }

    pub fn start_stk(&mut self, quote: &Id, attempt: StkAttempt) -> Result<(), Error> {
        self.quotes
            .get_mut(quote)
            .ok_or(Error::NotFound)?
            .start_stk(attempt)
    }

    pub fn acknowledge_stk(
        &mut self,
        quote: &Id,
        attempt: &Id,
        merchant: Id,
        checkout: Id,
    ) -> Result<(), Error> {
        self.quotes
            .get_mut(quote)
            .ok_or(Error::NotFound)?
            .acknowledge_stk(attempt, merchant, checkout)
    }

    pub fn observe(&mut self, observation: ReceiptObservation) {
        self.observations.push(observation);
    }

    fn allocated(&self, quote: &Id) -> Result<Amount, Error> {
        self.receipts
            .values()
            .filter(|r| r.allocation == Allocation::Quote(quote.clone()))
            .try_fold(Amount::new(0, Unit::KesMinor), |total, r| {
                total.checked_add(r.deposit.0.amount)
            })
    }

    pub fn confirm(&mut self, deposit: VerifiedDeposit) -> Result<DepositState, Error> {
        self.confirm_with_grace(deposit, 0)
    }

    pub(crate) fn hold_until(&mut self, id: &Id, until: Timestamp) -> Result<(), Error> {
        let quote = self.quotes.get_mut(id).ok_or(Error::NotFound)?;
        quote.terms.hold_until = quote.terms.hold_until.max(until);
        Ok(())
    }

    pub(crate) fn reserve_refund(&mut self, id: &Id) -> Result<(), Error> {
        let quote = self.quotes.get_mut(id).ok_or(Error::NotFound)?;
        if !matches!(
            quote.state,
            InboundState::UnderpaidHeld | InboundState::OverpaidHeld
        ) {
            return Err(Error::InvalidTransition);
        }
        quote.state = InboundState::RefundHeld;
        Ok(())
    }

    pub(crate) fn confirm_with_grace(
        &mut self,
        deposit: VerifiedDeposit,
        grace: u64,
    ) -> Result<DepositState, Error> {
        let evidence = &deposit.0;
        if self.receipts.contains_key(&evidence.receipt) {
            return Err(Error::DuplicateReceipt);
        }
        let mut allocation = Allocation::Exception;
        let mut state = DepositState::OrphanUnclaimed;
        let mut quote_update = None;
        if let Some(id) = &evidence.quote_id {
            let quote = self.quotes.get(id).ok_or(Error::NotFound)?;
            let rail_matches = match &evidence.rail {
                DepositRail::C2b { account_ref } => account_ref == &quote.account_ref,
                DepositRail::Stk { checkout_request } => quote
                    .attempts
                    .iter()
                    .any(|a| a.checkout_request.as_ref() == Some(checkout_request)),
            };
            if evidence.shortcode != quote.shortcode || !rail_matches {
                return Err(Error::EvidenceMismatch);
            }
            if matches!(quote.state, InboundState::Paid | InboundState::RefundHeld) {
                state = DepositState::OverpaidHeld;
            } else if !quote.accepts_receipt_time(evidence.received_at, grace)? {
                state = DepositState::LateHeld;
            } else if quote
                .terms
                .fx
                .as_ref()
                .is_some_and(|fx| evidence.verified_at >= fx.expires_at)
            {
                state = DepositState::FxExpiredHeld;
            } else {
                let total = self.allocated(id)?.checked_add(evidence.amount)?;
                allocation = Allocation::Quote(id.clone());
                let (deposit_state, quote_state) = match total
                    .value()
                    .cmp(&quote.terms.expected.value())
                {
                    std::cmp::Ordering::Less => {
                        (DepositState::UnderpaidHeld, InboundState::UnderpaidHeld)
                    }
                    std::cmp::Ordering::Greater => {
                        (DepositState::OverpaidHeld, InboundState::OverpaidHeld)
                    }
                    std::cmp::Ordering::Equal
                        if self.blocked || evidence.verified_at < quote.terms.hold_until =>
                    {
                        (DepositState::SettlementHeld, InboundState::SettlementHeld)
                    }
                    std::cmp::Ordering::Equal => (DepositState::Verified, InboundState::Verifying),
                };
                state = deposit_state;
                quote_update = Some((id.clone(), quote_state));
            }
        }
        // All fallible validation precedes any aggregate mutation.
        if let Some((id, state)) = quote_update {
            self.quotes.get_mut(&id).expect("validated quote").state = state;
        }
        self.receipts.insert(
            evidence.receipt.clone(),
            ConfirmedReceipt {
                deposit,
                allocation,
                state,
            },
        );
        Ok(state)
    }

    pub fn settle(&mut self, id: &Id, now: Timestamp) -> Result<SettlementEvent, Error> {
        let quote = self.quotes.get(id).ok_or(Error::NotFound)?;
        if matches!(quote.state, InboundState::Paid | InboundState::RefundHeld)
            || self.events.contains_key(id)
        {
            return Err(Error::InvalidTransition);
        }
        if self.allocated(id)? != quote.terms.expected {
            return Err(Error::AmountMismatch);
        }
        if quote
            .terms
            .fx
            .as_ref()
            .is_some_and(|fx| now >= fx.expires_at)
        {
            return Err(Error::FxExpired);
        }
        if self.blocked || now < quote.terms.hold_until {
            return Err(Error::SettlementHeld);
        }
        let mut receipt_ids = vec![];
        for (receipt, record) in &self.receipts {
            if record.allocation == Allocation::Quote(id.clone()) {
                if now < record.deposit.0.verified_at {
                    return Err(Error::EvidenceMismatch);
                }
                receipt_ids.push(receipt.clone());
            }
        }
        let event = SettlementEvent {
            quote_id: id.clone(),
            credit: quote.terms.credit,
            receipts: receipt_ids,
            settled_at: now,
        };
        self.quotes.get_mut(id).expect("validated quote").state = InboundState::Paid;
        for record in self.receipts.values_mut() {
            if record.allocation == Allocation::Quote(id.clone()) {
                record.state = DepositState::Settled;
            }
        }
        self.events.insert(id.clone(), event.clone());
        Ok(event)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "InboundState")]
enum StoredInboundState {
    Issued,
    StkSent,
    Verifying,
    UnderpaidHeld,
    OverpaidHeld,
    SettlementHeld,
    Paid,
    RefundHeld,
}
impl crate::storage::Stored for InboundState {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredInboundState::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "QuoteTerms")]
struct StoredQuoteTerms {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    expected: Amount,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    credit: Amount,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    quote_expires_at: Timestamp,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    fx: Option<FxSnapshot>,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    tariff: TariffSnapshot,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    hold_until: Timestamp,
}
impl crate::storage::Stored for QuoteTerms {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredQuoteTerms::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "AttemptState")]
enum StoredAttemptState {
    Dispatching,
    Sent,
    Unknown,
    Completed,
    Failed,
}
impl crate::storage::Stored for AttemptState {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredAttemptState::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "StkAttempt")]
struct StoredStkAttempt {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    id: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    started_at: Timestamp,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    next_poll_at: Timestamp,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    resend_after: Timestamp,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    state: AttemptState,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    caller_request: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    merchant_request: Option<Id>,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    checkout_request: Option<Id>,
}
impl crate::storage::Stored for StkAttempt {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredStkAttempt::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "InboundQuote")]
struct StoredInboundQuote {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    id: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    account_ref: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    shortcode: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    terms: QuoteTerms,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    state: InboundState,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    attempts: Vec<StkAttempt>,
}
impl crate::storage::Stored for InboundQuote {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredInboundQuote::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "DepositRail")]
enum StoredDepositRail {
    Stk {
        #[serde(deserialize_with = "crate::storage::deserialize")]
        checkout_request: Id,
    },
    C2b {
        #[serde(deserialize_with = "crate::storage::deserialize")]
        account_ref: Id,
    },
}
impl crate::storage::Stored for DepositRail {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredDepositRail::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "ReceiptObservation")]
struct StoredReceiptObservation {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    observation_id: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    claimed_receipt: ReceiptId,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    claimed_quote: Option<Id>,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    claimed_amount: Amount,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    rail: DepositRail,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    observed_at: Timestamp,
}
impl crate::storage::Stored for ReceiptObservation {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredReceiptObservation::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "EvidenceSource")]
enum StoredEvidenceSource {
    AuthenticatedProviderRecord(#[serde(deserialize_with = "crate::storage::deserialize")] Id),
    ReviewedStatement(#[serde(deserialize_with = "crate::storage::deserialize")] Id),
}
impl crate::storage::Stored for EvidenceSource {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredEvidenceSource::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "DepositAttestation")]
struct StoredDepositAttestation {
    #[serde(default)]
    payer: Option<VerifiedPayer>,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    source: EvidenceSource,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    receipt: ReceiptId,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    quote_id: Option<Id>,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    amount: Amount,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    shortcode: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    rail: DepositRail,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    received_at: Timestamp,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    verified_at: Timestamp,
}
impl crate::storage::Stored for DepositAttestation {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredDepositAttestation::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "VerifiedDeposit")]
struct StoredVerifiedDeposit(
    #[serde(deserialize_with = "crate::storage::deserialize")] DepositAttestation,
);
impl crate::storage::Stored for VerifiedDeposit {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredVerifiedDeposit::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "DepositState")]
enum StoredDepositState {
    UnderpaidHeld,
    OverpaidHeld,
    OrphanUnclaimed,
    LateHeld,
    FxExpiredHeld,
    SettlementHeld,
    Verified,
    Settled,
}
impl crate::storage::Stored for DepositState {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredDepositState::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "Allocation")]
enum StoredAllocation {
    Quote(#[serde(deserialize_with = "crate::storage::deserialize")] Id),
    Exception,
}
impl crate::storage::Stored for Allocation {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredAllocation::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "ConfirmedReceipt")]
struct StoredConfirmedReceipt {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    deposit: VerifiedDeposit,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    allocation: Allocation,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    state: DepositState,
}
impl crate::storage::Stored for ConfirmedReceipt {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredConfirmedReceipt::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "SettlementEvent")]
struct StoredSettlementEvent {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    quote_id: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    credit: Amount,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    receipts: Vec<ReceiptId>,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    settled_at: Timestamp,
}
impl crate::storage::Stored for SettlementEvent {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredSettlementEvent::deserialize(d)
    }
}

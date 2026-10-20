//! Durable dual-rail checkout. Dispatch is attempted once, after the preparation commit.
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::Arc,
};

use rsa::rand_core::{OsRng, RngCore};
use serde::Serialize;

use crate::{
    Error, Timestamp,
    amount::{Amount, TariffSnapshot, Unit},
    collection::{CollectionClient, StkAcknowledgment, StkSubmission, StkTransactionType},
    crypto::{Clock, Secret},
    identity::{Id, RequestKey, RequestPayload},
    inbound::{AttemptState, InboundQuote, InboundState, QuoteTerms, StkAttempt},
    repository::{Repository, RepositoryError, StkChange},
    transport::{ClientError, ErrorKind, HttpTransport},
};

#[derive(Debug)]
pub enum CheckoutError {
    Domain(Error),
    Repository(RepositoryError),
    UnsupportedUnit,
    InvalidConfiguration,
    Cooldown { retry_after: Timestamp },
    ReconciliationRequired,
    IdentityExhausted,
    Fx(crate::fx::FxError),
}
impl From<Error> for CheckoutError {
    fn from(e: Error) -> Self {
        Self::Domain(e)
    }
}
impl From<RepositoryError> for CheckoutError {
    fn from(e: RepositoryError) -> Self {
        Self::Repository(e)
    }
}
impl std::fmt::Display for CheckoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CheckoutError {}
pub type Result<T> = std::result::Result<T, CheckoutError>;

/// Provider query failure semantics must be verified for the deployed endpoint/environment.
#[derive(Clone, Debug, Default)]
pub struct StkReconciliationPolicy {
    final_failure_codes: BTreeSet<Id>,
}
impl StkReconciliationPolicy {
    pub fn verified_failure_codes(codes: impl IntoIterator<Item = Id>) -> Result<Self> {
        let final_failure_codes: BTreeSet<_> = codes.into_iter().collect();
        if final_failure_codes.iter().any(|code| code.as_str() == "0") {
            return Err(CheckoutError::InvalidConfiguration);
        }
        Ok(Self {
            final_failure_codes,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CheckoutConfig {
    pub shortcode: Id,
    pub ttl_seconds: u32,
    /// Explicit collection fee, separate from disbursement tariff schedules.
    pub tariff: TariffSnapshot,
    pub settlement_hold_seconds: u32,
    pub callback_url: Secret,
    pub stk_reconciliation: StkReconciliationPolicy,
}
#[derive(Clone, Debug)]
pub struct CreateCheckout {
    pub key: RequestKey,
    /// Exact gross M-Pesa collection amount. KES is represented in minor units.
    pub amount: Amount,
    pub phone: Option<Secret>,
}
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ManualInstructions {
    pub paybill: Id,
    pub account_ref: Id,
    pub amount: Amount,
    pub whole_kes: u64,
}
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Checkout {
    pub quote_id: Id,
    pub payment_state: InboundState,
    pub credit: Amount,
    pub attempt_id: Option<Id>,
    pub stk_status: Option<AttemptState>,
    pub checkout_request_id: Option<Id>,
    pub fallback: ManualInstructions,
    pub retry_after: Option<Timestamp>,
    pub expires_at: Timestamp,
}
impl Checkout {
    fn from_quote(q: &InboundQuote) -> Result<Self> {
        let a = q.attempts().last();
        Ok(Self {
            quote_id: q.id().clone(),
            payment_state: q.state(),
            credit: q.terms().credit,
            attempt_id: a.map(|a| a.id().clone()),
            stk_status: a.map(StkAttempt::state),
            checkout_request_id: a.and_then(|a| a.checkout_request().cloned()),
            fallback: ManualInstructions {
                paybill: q.shortcode().clone(),
                account_ref: q.account_ref().clone(),
                amount: q.terms().expected,
                whole_kes: q.terms().expected.whole_kes()?,
            },
            retry_after: a.map(StkAttempt::resend_after),
            expires_at: q.terms().quote_expires_at,
        })
    }
}

pub trait CheckoutIdentity: Send + Sync {
    fn id(&self) -> Result<Id>;
    fn account_ref(&self) -> Result<Id>;
}
pub struct RandomIdentity;
impl CheckoutIdentity for RandomIdentity {
    fn id(&self) -> Result<Id> {
        let mut bytes = [0u8; 16];
        OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| CheckoutError::IdentityExhausted)?;
        Ok(Id::new(
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        )?)
    }
    fn account_ref(&self) -> Result<Id> {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut value = String::with_capacity(8);
        while value.len() < 8 {
            let mut byte = [0u8];
            OsRng
                .try_fill_bytes(&mut byte)
                .map_err(|_| CheckoutError::IdentityExhausted)?;
            if byte[0] < 252 {
                value.push(ALPHABET[usize::from(byte[0] % 36)] as char);
            }
        }
        Ok(Id::new(value)?)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconciledAttempt {
    Failed,
    Completed,
    Unresolved,
}
pub trait CheckoutCollection: Send + Sync {
    fn shortcode(&self) -> &Id;
    fn submit(
        &self,
        request: StkSubmission,
    ) -> impl Future<Output = std::result::Result<StkAcknowledgment, ClientError>> + Send;
    /// Attempt status only: no settlement authority. Uncertainty must remain unresolved.
    fn reconcile(
        &self,
        attempt: &StkAttempt,
        policy: &StkReconciliationPolicy,
    ) -> impl Future<Output = ReconciledAttempt> + Send;
}
impl<T: HttpTransport, C: Clock + Send + Sync> CheckoutCollection for CollectionClient<T, C> {
    fn shortcode(&self) -> &Id {
        self.shortcode()
    }
    async fn submit(
        &self,
        request: StkSubmission,
    ) -> std::result::Result<StkAcknowledgment, ClientError> {
        self.submit_stk(&request).await
    }
    async fn reconcile(
        &self,
        attempt: &StkAttempt,
        policy: &StkReconciliationPolicy,
    ) -> ReconciledAttempt {
        let Some(checkout) = attempt.checkout_request() else {
            return ReconciledAttempt::Unresolved;
        };
        match self.query_stk(checkout).await {
            Ok(observation)
                if attempt.merchant_request() != Some(&observation.merchant_request_id) =>
            {
                ReconciledAttempt::Unresolved
            }
            Ok(observation) => match observation.result_code.as_str() {
                "0" => ReconciledAttempt::Completed,
                _ if policy
                    .final_failure_codes
                    .contains(&observation.result_code) =>
                {
                    ReconciledAttempt::Failed
                }
                _ => ReconciledAttempt::Unresolved,
            },
            Err(_) => ReconciledAttempt::Unresolved,
        }
    }
}

/// Manual checkout remains available when no STK transport is configured.
pub struct UnavailableCollection {
    pub shortcode: Id,
}
impl CheckoutCollection for UnavailableCollection {
    fn shortcode(&self) -> &Id {
        &self.shortcode
    }
    async fn submit(
        &self,
        _: StkSubmission,
    ) -> std::result::Result<StkAcknowledgment, ClientError> {
        Err(ClientError::new(ErrorKind::CapabilityDisabled))
    }
    async fn reconcile(&self, _: &StkAttempt, _: &StkReconciliationPolicy) -> ReconciledAttempt {
        ReconciledAttempt::Unresolved
    }
}

pub struct CheckoutService<P, C, I = RandomIdentity> {
    repository: Repository,
    collection: Arc<P>,
    clock: Arc<C>,
    identities: I,
    config: CheckoutConfig,
    sat_capability: bool,
    kes_capability: bool,
}
impl<P, C, I> CheckoutService<P, C, I>
where
    P: CheckoutCollection + 'static,
    C: Clock + Send + Sync + 'static,
    I: CheckoutIdentity,
{
    pub fn new(
        repository: Repository,
        collection: Arc<P>,
        clock: Arc<C>,
        identities: I,
        config: CheckoutConfig,
    ) -> Result<Self> {
        if config.ttl_seconds == 0
            || config.shortcode != *collection.shortcode()
            || !config
                .shortcode
                .as_str()
                .bytes()
                .all(|b| b.is_ascii_digit())
            || config.tariff.fee.unit() != Unit::KesMinor
        {
            return Err(CheckoutError::InvalidConfiguration);
        }
        Ok(Self {
            repository,
            collection,
            clock,
            identities,
            config,
            sat_capability: false,
            kes_capability: false,
        })
    }
    fn now(&self) -> Result<Timestamp> {
        Ok(Timestamp(
            u64::try_from(self.clock.now().timestamp()).map_err(|_| Error::Overflow)?,
        ))
    }
    pub async fn checkout(&self, quote: &Id) -> Result<Checkout> {
        Checkout::from_quote(&self.repository.quote(quote).await?.0)
    }
    /// Enable only after the deployed mint's CDK-SAT capability is independently verified.
    pub fn with_verified_sat_capability(mut self, enabled: bool) -> Self {
        self.sat_capability = enabled;
        self
    }
    /// CDK settings must not infer custom KES support from local amount arithmetic.
    pub fn with_verified_kes_capability(mut self, enabled: bool) -> Self {
        self.kes_capability = enabled;
        self
    }
    pub async fn enabled_units(&self, oracle: &impl crate::fx::FxOracle) -> Vec<Unit> {
        let mut units = Vec::new();
        if self.kes_capability {
            units.push(Unit::KesMinor);
        }
        if self.sat_capability
            && let Ok(now) = self.now()
            && let Ok(fx) = oracle.quote(now).await
            && let Ok(fresh_now) = self.now()
            && valid_fx(&fx, fresh_now).is_ok()
        {
            units.push(Unit::Sat);
        }
        units
    }
    pub async fn create(&self, request: CreateCheckout) -> Result<Checkout> {
        self.create_with_oracle(request, &crate::fx::OfflineOracle { snapshot: None })
            .await
    }
    pub async fn create_with_oracle(
        &self,
        request: CreateCheckout,
        oracle: &impl crate::fx::FxOracle,
    ) -> Result<Checkout> {
        if request.key.operation.as_str() != "inbound-create" {
            return Err(Error::EvidenceMismatch.into());
        }
        let now = self.now()?;
        if request.amount.value() == 0 {
            return Err(Error::InvalidAmount.into());
        }
        validate_phone(request.phone.as_ref())?;
        let payload = self.payload(
            &request.key,
            Id::new("new-inbound-quote")?,
            request.amount,
            request.phone.as_ref(),
        )?;
        if let Some(id) = self
            .repository
            .checkout_binding(&request.key, &payload)
            .await?
        {
            return self.checkout(&id).await;
        }
        let (expected, credit, fx) = match request.amount.unit() {
            Unit::KesMinor => {
                request.amount.whole_kes()?;
                (
                    request.amount,
                    request.amount.checked_sub(self.config.tariff.fee)?,
                    None,
                )
            }
            Unit::Sat => {
                if !self.sat_capability {
                    return Err(CheckoutError::UnsupportedUnit);
                }
                let fx = oracle.quote(now).await.map_err(CheckoutError::Fx)?;
                // Re-read time after I/O; a slow oracle cannot extend its own validity.
                valid_fx(&fx, self.now()?)?;
                let expected = fx
                    .backing_minor(request.amount)?
                    .checked_add(self.config.tariff.fee)?
                    .ceil_whole_kes()?;
                expected.whole_kes()?;
                (expected, request.amount, Some(fx))
            }
        };
        let expires = add(now, self.config.ttl_seconds)?;
        let terms = QuoteTerms {
            expected,
            credit,
            quote_expires_at: fx.as_ref().map_or(expires, |f| expires.min(f.expires_at)),
            fx,
            tariff: self.config.tariff.clone(),
            hold_until: add(now, self.config.settlement_hold_seconds)?,
        };
        if self.now()? >= terms.quote_expires_at {
            return Err(Error::QuoteExpired.into());
        }
        for _ in 0..32 {
            let q = InboundQuote::new(
                self.identities.id()?,
                self.identities.account_ref()?,
                self.config.shortcode.clone(),
                terms.clone(),
            )?;
            let attempt = request
                .phone
                .as_ref()
                .map(|_| self.attempt(now))
                .transpose()?;
            match self
                .repository
                .prepare_checkout(&request.key, &payload, &q, attempt.clone(), None, now)
                .await
            {
                Ok((id, created)) => {
                    // Take the committed response before scheduling network work, so fallback never waits for STK.
                    let response = self.checkout(&id).await?;
                    if created && let (Some(a), Some(phone)) = (attempt, request.phone) {
                        self.dispatch(q, a, phone);
                    }
                    return Ok(response);
                }
                Err(RepositoryError::ReferenceCollision) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(CheckoutError::IdentityExhausted)
    }
    pub async fn resend(&self, quote: &Id, key: RequestKey, phone: Secret) -> Result<Checkout> {
        if key.operation.as_str() != "inbound-resend" {
            return Err(Error::EvidenceMismatch.into());
        }
        validate_phone(Some(&phone))?;
        let (mut q, mut version) = self.repository.quote(quote).await?;
        if q.shortcode() != &self.config.shortcode {
            return Err(CheckoutError::InvalidConfiguration);
        }
        let payload = self.payload(&key, quote.clone(), q.terms().expected, Some(&phone))?;
        if let Some(id) = self.repository.checkout_binding(&key, &payload).await? {
            return self.checkout(&id).await;
        }
        let now = self.now()?;
        if let Some(last) = q.attempts().last()
            && now < last.resend_after()
        {
            return Err(CheckoutError::Cooldown {
                retry_after: last.resend_after(),
            });
        }
        if !matches!(q.state(), InboundState::Issued | InboundState::StkSent) {
            return Err(Error::InvalidTransition.into());
        }
        if now >= q.terms().quote_expires_at {
            return Err(Error::QuoteExpired.into());
        }
        for a in q.attempts() {
            match a.state() {
                AttemptState::Failed if a.checkout_request().is_none() => continue,
                AttemptState::Completed => return Err(CheckoutError::ReconciliationRequired),
                _ => {
                    let status = self
                        .collection
                        .reconcile(a, &self.config.stk_reconciliation)
                        .await;
                    let succeeded = match status {
                        ReconciledAttempt::Failed => false,
                        ReconciledAttempt::Completed => true,
                        ReconciledAttempt::Unresolved => {
                            return Err(CheckoutError::ReconciliationRequired);
                        }
                    };
                    if a.state() != AttemptState::Failed {
                        self.repository
                            .change_stk(
                                quote,
                                version,
                                StkChange::Finish {
                                    attempt: a.id().clone(),
                                    succeeded,
                                },
                                self.now()?,
                            )
                            .await?;
                        version += 1;
                    }
                    if succeeded {
                        return Err(CheckoutError::ReconciliationRequired);
                    }
                }
            }
        }
        q = self.repository.quote(quote).await?.0;
        // CAS uses the reconciled version; a concurrent attempt/receipt invalidates this preparation.
        let now = self.now()?;
        let a = self.attempt(now)?;
        let (id, created) = self
            .repository
            .prepare_checkout(&key, &payload, &q, Some(a.clone()), Some(version), now)
            .await?;
        let response = self.checkout(&id).await?;
        if created {
            self.dispatch(q, a, phone);
        }
        Ok(response)
    }
    fn attempt(&self, now: Timestamp) -> Result<StkAttempt> {
        Ok(StkAttempt::new(
            self.identities.id()?,
            self.identities.id()?,
            now,
        )?)
    }
    fn payload(
        &self,
        key: &RequestKey,
        quote: Id,
        amount: Amount,
        phone: Option<&Secret>,
    ) -> Result<RequestPayload> {
        let options = BTreeMap::from([
            (
                "phone".into(),
                phone.map(|p| p.expose().to_owned()).unwrap_or_default(),
            ),
            ("request_contract".into(), "inbound-v2".into()),
        ]);
        Ok(RequestPayload {
            operation: key.operation.clone(),
            quote_id: quote,
            destination: self.config.shortcode.clone(),
            amount,
            tariff_version: Id::new("inbound-request-v2")?,
            options,
        })
    }
    fn dispatch(&self, q: InboundQuote, a: StkAttempt, phone: Secret) {
        let repository = self.repository.clone();
        let collection = self.collection.clone();
        let clock = self.clock.clone();
        let request = StkSubmission {
            amount: q.terms().expected,
            phone,
            callback_url: self.config.callback_url.clone(),
            account_reference: q.account_ref().clone(),
            description: Secret::new("Mint deposit"),
            transaction_type: StkTransactionType::CustomerPayBillOnline,
        };
        tokio::spawn(async move {
            let Ok(seconds) = u64::try_from(clock.now().timestamp()) else {
                return;
            };
            if Timestamp(seconds) >= q.terms().quote_expires_at
                || q.terms()
                    .fx
                    .as_ref()
                    .is_some_and(|f| Timestamp(seconds) >= f.expires_at)
            {
                return;
            }
            let change = match collection.submit(request).await {
                Ok(ack) => StkChange::RecordAcknowledgment {
                    attempt: a.id().clone(),
                    merchant: ack.merchant_request_id,
                    checkout: ack.checkout_request_id,
                },
                Err(e)
                    if matches!(
                        e.kind,
                        ErrorKind::Config | ErrorKind::Input | ErrorKind::CapabilityDisabled
                    ) =>
                {
                    StkChange::Finish {
                        attempt: a.id().clone(),
                        succeeded: false,
                    }
                }
                Err(_) => StkChange::Unknown(a.id().clone()),
            };
            // Concurrent callbacks/settlement can win the CAS. Retry only the local write, never the submission.
            for _ in 0..8 {
                let Ok((_, version)) = repository.quote(q.id()).await else {
                    return;
                };
                let Ok(seconds) = u64::try_from(clock.now().timestamp()) else {
                    return;
                };
                match repository
                    .change_stk(q.id(), version, change.clone(), Timestamp(seconds))
                    .await
                {
                    Err(RepositoryError::Conflict) => continue,
                    _ => return,
                }
            }
        });
    }
}
fn add(now: Timestamp, seconds: u32) -> Result<Timestamp> {
    Ok(Timestamp(
        now.0
            .checked_add(u64::from(seconds))
            .ok_or(Error::Overflow)?,
    ))
}
fn validate_phone(phone: Option<&Secret>) -> Result<()> {
    if let Some(p) = phone
        && (p.expose().len() != 12
            || !p.expose().starts_with("254")
            || !p.expose().bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(Error::InvalidIdentity.into());
    }
    Ok(())
}

fn valid_fx(fx: &crate::amount::FxSnapshot, now: Timestamp) -> Result<()> {
    fx.validate_at(now)?;
    if fx.expires_at.0 > now.0.checked_add(300).ok_or(Error::Overflow)? {
        return Err(CheckoutError::InvalidConfiguration);
    }
    Ok(())
}

use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use super::{DispatchRecord, Payout};
use crate::{
    Error, Timestamp,
    crypto::{Clock, Secret},
    identity::{Id, RequestKey, RequestPayload},
    liquidity::{SweepClaim, SweepConfig},
    payout::{
        Destination, PayoutAcknowledgment, PayoutClient, PayoutPersistence, PayoutRequest,
        PreparedPayout,
    },
    repository::{Repository, RepositoryError},
    transport::{ClientError, ErrorKind, HttpTransport},
    valuation::PaymentQuote,
};

#[derive(Debug)]
pub enum DispatchError {
    Domain(Error),
    Repository(RepositoryError),
    Transport(ClientError),
}
impl From<Error> for DispatchError {
    fn from(e: Error) -> Self {
        Self::Domain(e)
    }
}
impl From<RepositoryError> for DispatchError {
    fn from(e: RepositoryError) -> Self {
        Self::Repository(e)
    }
}
impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DispatchError {}

#[derive(Clone, Debug)]
pub struct DispatchConfig {
    pub b2c_result_url: Secret,
    pub b2c_timeout_url: Secret,
    pub b2b_result_url: Secret,
    pub b2b_timeout_url: Secret,
    pub first_check_seconds: u32,
    pub manual_after_seconds: u32,
}
#[derive(Clone, Debug)]
pub struct MakePayment {
    /// Caller-scoped stable request key; operation must be `payout`.
    pub key: RequestKey,
    pub quote_id: Id,
}

pub struct OutboundPayments<T, C> {
    repository: Repository,
    client: PayoutClient<T>,
    clock: C,
    config: DispatchConfig,
}
impl<T: HttpTransport, C: Clock + Send + Sync> OutboundPayments<T, C> {
    pub fn new(
        repository: Repository,
        client: PayoutClient<T>,
        clock: C,
        config: DispatchConfig,
    ) -> Result<Self, DispatchError> {
        if config.first_check_seconds == 0
            || config.manual_after_seconds < config.first_check_seconds
        {
            return Err(Error::InvalidTransition.into());
        }
        Ok(Self {
            repository,
            client,
            clock,
            config,
        })
    }

    pub async fn make_payment(&self, request: MakePayment) -> Result<Payout, DispatchError> {
        if request.key.operation.as_str() != "payout" {
            return Err(Error::EvidenceMismatch.into());
        }
        let quote = self.repository.payment_quote(&request.quote_id).await?;
        let payload = RequestPayload {
            operation: request.key.operation.clone(),
            quote_id: quote.id().clone(),
            destination: quote.destination().clone(),
            amount: quote.principal(),
            tariff_version: quote.tariff().version.clone(),
            options: BTreeMap::new(),
        };
        // Replays survive quote expiry and configuration/credential reloads without rebuilding wire data.
        if let Some(p) = self
            .repository
            .payout_replay(&request.key, &payload)
            .await?
        {
            return Ok(p);
        }
        let operation_id = operation_id(&request.key)?;
        let destination =
            Destination::parse(quote.destination().as_str()).map_err(DispatchError::Transport)?;
        let (result_url, timeout_url) = if matches!(destination, Destination::Phone(_)) {
            (&self.config.b2c_result_url, &self.config.b2c_timeout_url)
        } else {
            (&self.config.b2b_result_url, &self.config.b2b_timeout_url)
        };
        let hooks = Hooks {
            repository: &self.repository,
            key: &request.key,
            quote: Some(&quote),
            sweep: None,
            payload: &payload,
            operation_id: &operation_id,
            clock: &self.clock,
            config: &self.config,
            claimed: AtomicBool::new(false),
            error: Mutex::new(None),
        };
        let result = self
            .client
            .submit(
                &PayoutRequest {
                    caller_originator_conversation_id: operation_id.clone(),
                    destination,
                    amount: quote.principal(),
                    result_url: result_url.clone(),
                    timeout_url: timeout_url.clone(),
                },
                &hooks,
            )
            .await;
        self.complete(&hooks, result).await
    }

    /// None keeps sweeps disabled and refreshes actionable operator alerts.
    /// Use a separately configured source-account client with explicitly verified links.
    pub async fn sweep_if_needed(
        &self,
        config: Option<&SweepConfig>,
    ) -> Result<Option<Payout>, DispatchError> {
        let now = now(&self.clock)?;
        self.repository.refresh_liquidity_alerts(now).await?;
        let Some(config) = config else {
            return Ok(None);
        };
        let Some(snapshot) = self.repository.sweep_candidate(now).await? else {
            return Ok(None);
        };
        let key = RequestKey {
            caller: Id::new("liquidity")?,
            operation: Id::new("sweep")?,
            key: snapshot.clone(),
        };
        let destination = Id::new(format!("PAYBILL:{}:FLOAT", config.utility.as_str()))?;
        let payload = RequestPayload {
            operation: key.operation.clone(),
            quote_id: snapshot.clone(),
            destination: destination.clone(),
            amount: config.amount,
            tariff_version: config.tariff.version.clone(),
            options: BTreeMap::new(),
        };
        if let Some(p) = self.repository.payout_replay(&key, &payload).await? {
            return Ok(Some(p));
        }
        let operation_id = operation_id(&key)?;
        let hooks = Hooks {
            repository: &self.repository,
            key: &key,
            quote: None,
            sweep: Some(SweepClaim {
                config,
                snapshot: &snapshot,
            }),
            payload: &payload,
            operation_id: &operation_id,
            clock: &self.clock,
            config: &self.config,
            claimed: AtomicBool::new(false),
            error: Mutex::new(None),
        };
        let result = self
            .client
            .sweep(
                &PayoutRequest {
                    caller_originator_conversation_id: operation_id.clone(),
                    destination: Destination::parse(destination.as_str())
                        .map_err(DispatchError::Transport)?,
                    amount: config.amount,
                    result_url: self.config.b2b_result_url.clone(),
                    timeout_url: self.config.b2b_timeout_url.clone(),
                },
                &hooks,
            )
            .await;
        Ok(Some(self.complete(&hooks, result).await?))
    }

    async fn complete(
        &self,
        hooks: &Hooks<'_, C>,
        result: Result<PayoutAcknowledgment, crate::payout::PayoutError>,
    ) -> Result<Payout, DispatchError> {
        if let Some(error) = hooks.error.lock().expect("dispatch hook error").take() {
            return Err(error.into());
        }
        if let Err(error) = result {
            if hooks.claimed.load(Ordering::SeqCst) {
                // Proven transport rejection remains an observation until the final evidence boundary.
                self.repository
                    .record_dispatch_outcome(
                        hooks.operation_id,
                        None,
                        &format!("{:?}", error.disposition),
                        &error.source.outcome,
                        now(&self.clock)?,
                    )
                    .await?;
            } else {
                return self
                    .repository
                    .payout_replay(hooks.key, hooks.payload)
                    .await?
                    .ok_or(DispatchError::Transport(error.source));
            }
        }
        Ok(self.repository.payout(hooks.operation_id).await?.0)
    }
}

fn now(clock: &impl Clock) -> Result<Timestamp, Error> {
    Ok(Timestamp(
        u64::try_from(clock.now().timestamp()).map_err(|_| Error::InvalidAmount)?,
    ))
}
fn operation_id(key: &RequestKey) -> Result<Id, Error> {
    let mut h = Sha256::new();
    h.update(b"daraja.payout.operation.v1\0");
    for field in [&key.caller, &key.operation, &key.key] {
        h.update((field.as_str().len() as u64).to_be_bytes());
        h.update(field.as_str().as_bytes());
    }
    let mut bytes: [u8; 32] = h.finalize().into();
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes[..16].iter().map(|b| format!("{b:02x}")).collect();
    Id::new(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
struct Hooks<'a, C> {
    repository: &'a Repository,
    key: &'a RequestKey,
    quote: Option<&'a PaymentQuote>,
    sweep: Option<SweepClaim<'a>>,
    payload: &'a RequestPayload,
    operation_id: &'a Id,
    clock: &'a C,
    config: &'a DispatchConfig,
    claimed: AtomicBool,
    error: Mutex<Option<RepositoryError>>,
}
impl<C: Clock + Send + Sync> Hooks<'_, C> {
    fn capture<V>(&self, result: Result<V, RepositoryError>) -> Result<V, ClientError> {
        result.map_err(|e| {
            *self.error.lock().expect("dispatch hook error") = Some(e);
            ClientError::new(ErrorKind::Persistence)
        })
    }
}
impl<C: Clock + Send + Sync> PayoutPersistence for Hooks<'_, C> {
    async fn prepare(&self, prepared: &PreparedPayout) -> Result<(), ClientError> {
        let build = || -> Result<Payout, RepositoryError> {
            let now = now(self.clock)?;
            let tariff = self
                .quote
                .map(|q| q.tariff())
                .or_else(|| self.sweep.map(|s| &s.config.tariff))
                .ok_or(Error::EvidenceMismatch)?;
            let mut dispatch = DispatchRecord::new(
                self.operation_id.clone(),
                self.payload.clone(),
                prepared.wire_body.expose().as_bytes().to_vec(),
                tariff.clone(),
                self.payload.amount.checked_add(tariff.fee)?,
            )?;
            if let Some(quote) = self.quote {
                dispatch = dispatch.with_quote(quote.clone(), now)?;
            }
            Ok(Payout::dispatching(
                dispatch,
                self.operation_id.clone(),
                now,
                Timestamp(
                    now.0
                        .checked_add(u64::from(self.config.first_check_seconds))
                        .ok_or(Error::Overflow)?,
                ),
                Timestamp(
                    now.0
                        .checked_add(u64::from(self.config.manual_after_seconds))
                        .ok_or(Error::Overflow)?,
                ),
            )?)
        };
        let payout = self.capture(build())?;
        let (_, claimed) = self.capture(
            self.repository
                .prepare_payout_claim(
                    self.key,
                    &payout,
                    Some(prepared.endpoint.as_str()),
                    self.sweep,
                )
                .await,
        )?;
        self.claimed.store(claimed, Ordering::SeqCst);
        if !claimed {
            return Err(ClientError::new(ErrorKind::Persistence));
        }
        Ok(())
    }
    async fn before_send(&self) -> Result<(), ClientError> {
        let now = self.capture(now(self.clock).map_err(RepositoryError::from))?;
        self.capture(
            self.repository
                .recheck_dispatch_float(self.operation_id, now)
                .await,
        )
    }
    async fn acknowledge(&self, ack: &PayoutAcknowledgment) -> Result<(), ClientError> {
        let now = self.capture(now(self.clock).map_err(RepositoryError::from))?;
        self.capture(
            self.repository
                .record_dispatch_outcome(
                    self.operation_id,
                    Some((
                        ack.provider.originator_conversation_id.clone(),
                        ack.provider.conversation_id.clone(),
                    )),
                    "Acknowledged",
                    &ack.provider.outcome,
                    now,
                )
                .await,
        )
    }
}

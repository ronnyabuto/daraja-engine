//! Separate account observations and conservative local float accounting.
use std::{future::Future, sync::Mutex};

use crate::{
    Error, Timestamp,
    amount::Amount,
    collection::StatusAcknowledgment,
    crypto::{Clock, Secret},
    identity::Id,
    payout::{BalanceAccount, BalanceClient, BalancePersistence, BalancePrepared, BalanceRequest},
    repository::{Repository, RepositoryError},
    transport::{ClientError, ErrorKind, HttpTransport},
};

type Result<T> = std::result::Result<T, RepositoryError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountRole {
    Collection,
    Disbursement,
}
impl AccountRole {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Collection => "collection",
            Self::Disbursement => "disbursement",
        }
    }
}
#[derive(Clone, Debug)]
pub struct LiquidityPolicy {
    pub collection: Id,
    pub disbursement: Id,
    pub minimum: Amount,
    pub max_age_seconds: u32,
    pub poll_interval_seconds: u32,
}
impl LiquidityPolicy {
    pub(crate) fn validate(&self) -> Result<()> {
        self.minimum.require_kes()?;
        for account in [&self.collection, &self.disbursement] {
            if !(5..=6).contains(&account.as_str().len())
                || !account.as_str().bytes().all(|b| b.is_ascii_digit())
                || account.as_str().starts_with('0')
            {
                return Err(Error::InvalidIdentity.into());
            }
        }
        if self.collection == self.disbursement
            || self.max_age_seconds == 0
            || self.poll_interval_seconds == 0
        {
            return Err(Error::InvalidAmount.into());
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct BalanceTarget {
    pub query_id: Id,
    pub role: AccountRole,
    pub account: Id,
    pub requested_at: Timestamp,
    /// Correlated admitted callbacks remain untrusted. Early replies survive acknowledgment races.
    pub callbacks: Vec<Secret>,
}
#[derive(Debug)]
pub struct BalanceEvidence {
    pub query_id: Id,
    pub account: Id,
    pub role: AccountRole,
    pub available: Amount,
    pub evidence_record: Id,
}
/// Verify the account's available KES at or after this query's request time.
/// Correlation or a callback secret alone is insufficient provenance. Implementations
/// must validate the provider's account/available-balance fields, not total or credit limits.
pub trait BalanceVerifier: Send + Sync {
    fn evidence(
        &self,
        target: &BalanceTarget,
        now: Timestamp,
    ) -> impl Future<Output = Option<BalanceEvidence>> + Send;
}
pub struct UnavailableBalanceVerifier;
impl BalanceVerifier for UnavailableBalanceVerifier {
    async fn evidence(&self, _: &BalanceTarget, _: Timestamp) -> Option<BalanceEvidence> {
        None
    }
}

#[derive(Debug)]
pub struct FloatStatus {
    pub balance: Option<Amount>,
    pub age_seconds: Option<u64>,
    pub pending_liability: Amount,
    pub quote_reservations: Amount,
    pub available: Option<Amount>,
    pub alert: bool,
}
impl FloatStatus {
    pub fn operator_instruction(&self) -> Option<&'static str> {
        self.alert.then_some("Disbursement liquidity rebalancing in progress. Verify the disbursement account and initiate an operator bank/PesaLink transfer; collection funds are separate.")
    }
}

pub struct BalanceMonitor<T, C, V> {
    pub repository: Repository,
    pub collection: BalanceClient<T, C>,
    pub disbursement: BalanceClient<T, C>,
    pub clock: C,
    pub verifier: V,
    pub result_url: Secret,
    pub timeout_url: Secret,
}
impl<T: HttpTransport, C: Clock + Send + Sync, V: BalanceVerifier> BalanceMonitor<T, C, V> {
    fn now(&self) -> Result<Timestamp> {
        Ok(Timestamp(
            self.clock
                .now()
                .timestamp()
                .try_into()
                .map_err(|_| Error::Overflow)?,
        ))
    }
    /// Call periodically from the supervisor. The pre-I/O query transaction consumes
    /// the interval even on timeout, cancellation or restart; old requests are never resent.
    pub async fn run_once(&self) -> Result<()> {
        for role in [AccountRole::Collection, AccountRole::Disbursement] {
            let policy = self.repository.liquidity_policy().await?;
            let account = match role {
                AccountRole::Collection => policy.collection,
                AccountRole::Disbursement => policy.disbursement,
            };
            let client = match role {
                AccountRole::Collection => &self.collection,
                AccountRole::Disbursement => &self.disbursement,
            };
            // The durable writer check in prepare resolves concurrent monitor races.
            let request = BalanceRequest {
                local_query_id: query_id()?,
                account: BalanceAccount::Shortcode(account),
                result_url: self.result_url.clone(),
                timeout_url: self.timeout_url.clone(),
            };
            let hooks = BalanceHooks {
                repository: &self.repository,
                role,
                error: Mutex::new(None),
            };
            let _ = client.request(&request, &hooks).await;
            if let Some(error) = hooks.error.lock().expect("balance hook error").take() {
                return Err(error);
            }
            let now = self.now()?;
            for target in self.repository.balance_targets(role).await? {
                if let Some(evidence) = self.verifier.evidence(&target, now).await {
                    self.repository
                        .record_verified_balance(&target.query_id, evidence, now)
                        .await?;
                }
            }
        }
        self.repository
            .refresh_liquidity_alerts(self.now()?)
            .await?;
        Ok(())
    }
}
struct BalanceHooks<'a> {
    repository: &'a Repository,
    role: AccountRole,
    error: Mutex<Option<RepositoryError>>,
}
impl BalanceHooks<'_> {
    fn capture(&self, result: Result<()>) -> std::result::Result<(), ClientError> {
        result.map_err(|error| {
            if !matches!(error, RepositoryError::Conflict) {
                *self.error.lock().expect("balance hook error") = Some(error);
            }
            ClientError::new(ErrorKind::Persistence)
        })
    }
}
impl BalancePersistence for BalanceHooks<'_> {
    async fn prepare(&self, record: BalancePrepared<'_>) -> std::result::Result<(), ClientError> {
        self.capture(
            self.repository
                .prepare_balance_query(self.role, record)
                .await,
        )
    }
    async fn acknowledge(
        &self,
        query: &Id,
        ack: &StatusAcknowledgment,
    ) -> std::result::Result<(), ClientError> {
        self.repository
            .acknowledge_balance(query, ack)
            .await
            .map_err(|error| {
                *self.error.lock().expect("balance hook error") = Some(error);
                ClientError::new(ErrorKind::Persistence)
            })
    }
}

fn query_id() -> Result<Id> {
    use rsa::rand_core::{OsRng, RngCore};
    let mut bytes = [0u8; 16];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| Error::InvalidIdentity)?;
    Ok(Id::new(
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
    )?)
}

/// Explicit opt-in; transport capabilities independently attest the exact command,
/// caller-ID support and source/utility link for this endpoint/environment.
#[derive(Clone, Debug)]
pub struct SweepConfig {
    pub source: Id,
    pub utility: Id,
    pub amount: Amount,
    pub tariff: crate::amount::TariffSnapshot,
    pub max_daily: Amount,
    pub cooldown_seconds: u32,
}
#[derive(Clone, Copy)]
pub(crate) struct SweepClaim<'a> {
    pub config: &'a SweepConfig,
    pub snapshot: &'a Id,
}

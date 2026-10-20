//! Durable inbound work. Query observations and independent value evidence are separate gates.
use std::{future::Future, time::Duration};

use crate::{
    Error, Timestamp,
    amount::Amount,
    collection::{
        CollectionClient, StatusAcknowledgment, StatusIdentifierType, StatusLookup,
        StatusPersistence, StatusPrepared, StatusRequest, StatusResult, TransactionStatusClient,
    },
    crypto::{Clock, Secret},
    identity::{Id, ReceiptId},
    inbound::{
        DepositAttestation, DepositRail, InboundQuote, SettlementEvent, StkAttempt, VerifiedDeposit,
    },
    repository::{JobClaim, Repository, RepositoryError},
    transport::{ClientError, ErrorKind, HttpTransport},
};

type Result<T> = std::result::Result<T, RepositoryError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryState {
    Success,
    Unresolved,
    Throttled,
}

pub trait InboundQueries: Send + Sync {
    fn stk(&self, attempt: &StkAttempt) -> impl Future<Output = QueryState> + Send;
    fn c2b(
        &self,
        query_id: &Id,
        receipt: &ReceiptId,
        amount: Option<Amount>,
    ) -> impl Future<Output = QueryState> + Send;
}

/// Only an authenticated provider record or reviewed statement may implement this boundary.
/// Callback fields and query success cannot be copied into an independent attestation.
pub trait DepositVerifier: Send + Sync {
    fn evidence(
        &self,
        target: &VerificationTarget,
        now: Timestamp,
    ) -> impl Future<Output = Option<DepositAttestation>> + Send;
}
pub struct UnavailableVerifier;
impl DepositVerifier for UnavailableVerifier {
    async fn evidence(&self, _: &VerificationTarget, _: Timestamp) -> Option<DepositAttestation> {
        None
    }
}

#[derive(Clone, Debug)]
pub struct VerificationTarget {
    pub quote: Option<InboundQuote>,
    pub shortcode: Id,
    pub rail: DepositRail,
    pub claimed_receipt: Option<ReceiptId>,
    pub claimed_amount: Option<Amount>,
    pub claimed_received_at: Option<Timestamp>,
}
impl VerificationTarget {
    fn accepts(&self, e: &DepositAttestation, now: Timestamp) -> bool {
        e.quote_id.as_ref() == self.quote.as_ref().map(InboundQuote::id)
            && e.shortcode == self.shortcode
            && e.rail == self.rail
            && e.verified_at <= now
            && e.received_at <= e.verified_at
            && self
                .claimed_receipt
                .as_ref()
                .is_none_or(|r| r == &e.receipt)
            && self.claimed_amount.is_none_or(|a| a == e.amount)
            && self.claimed_received_at.is_none_or(|t| t == e.received_at)
            && match (&self.rail, &self.quote) {
                (DepositRail::Stk { checkout_request }, Some(q)) => q.attempts().iter().any(|a| {
                    a.checkout_request() == Some(checkout_request)
                        && e.received_at >= a.started_at()
                }),
                (DepositRail::C2b { account_ref }, Some(q)) => account_ref == q.account_ref(),
                (DepositRail::C2b { .. }, None) => true,
                _ => false,
            }
    }
}

pub struct InboundReconciler<Q, V, C> {
    pub repository: Repository,
    pub queries: Q,
    pub verifier: V,
    pub clock: C,
    pub shortcode: Id,
}
impl<Q: InboundQueries, V: DepositVerifier, C: Clock> InboundReconciler<Q, V, C> {
    fn now(&self) -> Result<Timestamp> {
        Ok(Timestamp(
            self.clock
                .now()
                .timestamp()
                .try_into()
                .map_err(|_| Error::Overflow)?,
        ))
    }
    /// Runs at most one leased job. Supervisors should sleep when idle and bound concurrency.
    pub async fn run_once(&self) -> Result<bool> {
        let now = self.now()?;
        let Some(claim) = self.repository.claim_inbound_job(now, 30).await? else {
            return Ok(false);
        };
        let Some((target, attempt)) = self.target(&claim).await? else {
            self.retry(&claim, now, "NO_PAYMENT_CLAIM", false, None)
                .await?;
            return Ok(true);
        };
        if let Some(q) = &target.quote
            && self
                .repository
                .settle_inbound_hold(&claim, q.id(), self.now()?)
                .await?
        {
            return Ok(true);
        }
        let probe = async {
            let query = match &target.rail {
                DepositRail::Stk { .. } => match &attempt {
                    Some(a) => self.queries.stk(a).await,
                    None => QueryState::Unresolved,
                },
                DepositRail::C2b { .. } => match &target.claimed_receipt {
                    Some(r) => self.queries.c2b(&claim.id, r, target.claimed_amount).await,
                    None => QueryState::Unresolved,
                },
            };
            let evidence = if query == QueryState::Success {
                self.verifier.evidence(&target, self.now()?).await
            } else {
                None
            };
            Ok::<_, RepositoryError>((query, evidence))
        };
        let result = tokio::time::timeout(Duration::from_secs(20), probe).await;
        let now = self.now()?;
        let (query, evidence) = match result {
            Ok(result) => result?,
            Err(_) => (QueryState::Unresolved, None),
        };
        if let Some(evidence) = evidence {
            if target.accepts(&evidence, now) {
                let verified = VerifiedDeposit::attest(evidence)?;
                self.repository
                    .apply_inbound_evidence(&claim, verified, now)
                    .await?;
                return Ok(true);
            }
            self.retry(&claim, now, "EVIDENCE_MISMATCH", false, attempt.as_ref())
                .await?;
        } else {
            self.retry(
                &claim,
                now,
                if query == QueryState::Success {
                    "INDEPENDENT_FIELDS_UNAVAILABLE"
                } else {
                    "QUERY_UNRESOLVED"
                },
                query == QueryState::Throttled,
                attempt.as_ref(),
            )
            .await?;
        }
        Ok(true)
    }
    async fn retry(
        &self,
        claim: &JobClaim,
        now: Timestamp,
        reason: &str,
        throttled: bool,
        attempt: Option<&StkAttempt>,
    ) -> Result<()> {
        let spike_retry = throttled && claim.retry_count < 5;
        let advance = claim.kind == "POLL_STK" && !spike_retry && claim.poll_slot < 3;
        let due = if spike_retry {
            now.0
                .checked_add((2u64 << claim.retry_count.min(4)).min(30))
        } else if claim.kind == "POLL_STK" && claim.poll_slot < 2 {
            let offset = [35, 65, 110][(claim.poll_slot + 1) as usize];
            let absolute = attempt.and_then(|a| a.started_at().0.checked_add(offset));
            absolute.map(|t| t.max(now.0.saturating_add(1)))
        } else {
            now.0.checked_add(300)
        }
        .ok_or(Error::Overflow)?;
        self.repository
            .finish_inbound_job(
                claim,
                now,
                Some(Timestamp(due)),
                reason,
                advance,
                spike_retry,
            )
            .await
    }
    async fn target(
        &self,
        claim: &JobClaim,
    ) -> Result<Option<(VerificationTarget, Option<StkAttempt>)>> {
        if claim.kind == "POLL_STK" {
            let (id, a) = self.repository.attempt(&claim.subject).await?;
            let (q, _) = self.repository.quote(&id).await?;
            let Some(checkout) = a.checkout_request().cloned() else {
                return Ok(None);
            };
            let target = VerificationTarget {
                shortcode: q.shortcode().clone(),
                quote: Some(q),
                rail: DepositRail::Stk {
                    checkout_request: checkout,
                },
                claimed_receipt: None,
                claimed_amount: None,
                claimed_received_at: None,
            };
            return Ok((target.shortcode == self.shortcode).then_some((target, Some(a))));
        }
        if claim.kind == "VERIFY_RECEIPT" {
            let o = self.repository.observation(&claim.subject).await?;
            let quote = match &o.claimed_quote {
                Some(id) => Some(self.repository.quote(id).await?.0),
                None => None,
            };
            let attempt = match (&o.rail, &quote) {
                (DepositRail::Stk { checkout_request }, Some(q)) => q
                    .attempts()
                    .iter()
                    .find(|a| a.checkout_request() == Some(checkout_request))
                    .cloned(),
                _ => None,
            };
            return Ok(Some((
                VerificationTarget {
                    quote,
                    shortcode: self.shortcode.clone(),
                    rail: o.rail,
                    claimed_receipt: Some(o.claimed_receipt),
                    claimed_amount: Some(o.claimed_amount),
                    claimed_received_at: None,
                },
                attempt,
            )));
        }
        let (endpoint, quote, body, _) = self.repository.inbound_callback(&claim.subject).await?;
        let value =
            crate::transport::strict_json(body.expose()).map_err(|_| RepositoryError::Encoding)?;
        let quote = match quote {
            Some(id) => Some(self.repository.quote(&id).await?.0),
            None => None,
        };
        let parsed = callback_target(&endpoint, quote, &self.shortcode, &value);
        Ok(parsed)
    }
}

fn field(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(crate::transport::scalar)
}
fn received(text: &str) -> Option<Timestamp> {
    let date = chrono::NaiveDateTime::parse_from_str(text, "%Y%m%d%H%M%S").ok()?;
    let eat = chrono::FixedOffset::east_opt(3 * 3600)?;
    let utc = date.and_local_timezone(eat).single()?.timestamp();
    Some(Timestamp(utc.try_into().ok()?))
}
fn callback_target(
    endpoint: &str,
    quote: Option<InboundQuote>,
    shortcode: &Id,
    v: &serde_json::Value,
) -> Option<(VerificationTarget, Option<StkAttempt>)> {
    let (rail, receipt, amount, time, attempt) = if endpoint == "/callbacks/stk/{secret}" {
        let c = v.pointer("/Body/stkCallback")?;
        if field(c, "ResultCode")? != "0" {
            return None;
        }
        let checkout = Id::new(field(c, "CheckoutRequestID")?).ok()?;
        let merchant = Id::new(field(c, "MerchantRequestID")?).ok()?;
        let a = quote
            .as_ref()?
            .attempts()
            .iter()
            .find(|a| {
                a.checkout_request() == Some(&checkout) && a.merchant_request() == Some(&merchant)
            })?
            .clone();
        let items = c.pointer("/CallbackMetadata/Item")?.as_array()?;
        let get = |name: &str| {
            items
                .iter()
                .find(|i| i.get("Name").and_then(serde_json::Value::as_str) == Some(name))?
                .get("Value")
        };
        (
            DepositRail::Stk {
                checkout_request: checkout,
            },
            ReceiptId::new(&crate::transport::scalar(get("MpesaReceiptNumber")?)?).ok()?,
            crate::collection::parse_kes(get("Amount")?).ok()?,
            received(&crate::transport::scalar(get("TransactionDate")?)?)?,
            Some(a),
        )
    } else if endpoint == "/callbacks/c2b/confirmation/{secret}" {
        if field(v, "BusinessShortCode")? != shortcode.as_str() {
            return None;
        }
        (
            DepositRail::C2b {
                account_ref: Id::new(field(v, "BillRefNumber")?).ok()?,
            },
            ReceiptId::new(&field(v, "TransID")?).ok()?,
            crate::collection::parse_kes(v.get("TransAmount")?).ok()?,
            received(&field(v, "TransTime")?)?,
            None,
        )
    } else {
        return None;
    };
    if quote.as_ref().is_some_and(|q| q.shortcode() != shortcode) {
        return None;
    }
    Some((
        VerificationTarget {
            quote,
            shortcode: shortcode.clone(),
            rail,
            claimed_receipt: Some(receipt),
            claimed_amount: Some(amount),
            claimed_received_at: Some(time),
        },
        attempt,
    ))
}

/// Live transport adapter. It obtains observations only; settlement still needs a DepositVerifier.
pub struct ProviderQueries<T, C, U, D> {
    pub collection: CollectionClient<T, C>,
    pub status: TransactionStatusClient<U, D>,
    pub repository: Repository,
    pub clock: C,
    pub status_config: InboundStatusConfig,
}
pub struct InboundStatusConfig {
    pub initiator: Secret,
    pub security_credential: Secret,
    pub shortcode: Id,
    pub result_url: Secret,
    pub timeout_url: Secret,
}
impl<T: HttpTransport, C: Clock + Send + Sync, U: HttpTransport, D: Clock + Send + Sync>
    InboundQueries for ProviderQueries<T, C, U, D>
{
    async fn stk(&self, attempt: &StkAttempt) -> QueryState {
        let Some(checkout) = attempt.checkout_request() else {
            return QueryState::Unresolved;
        };
        match self.collection.query_stk(checkout).await {
            Ok(o)
                if o.result_code.as_str() == "0"
                    && Some(&o.merchant_request_id) == attempt.merchant_request() =>
            {
                QueryState::Success
            }
            Err(e) if e.kind == ErrorKind::Throttled => QueryState::Throttled,
            _ => QueryState::Unresolved,
        }
    }
    async fn c2b(&self, query_id: &Id, receipt: &ReceiptId, amount: Option<Amount>) -> QueryState {
        let Ok(seconds) = self.clock.now().timestamp().try_into() else {
            return QueryState::Unresolved;
        };
        match self
            .repository
            .status_prepared(query_id, receipt, Timestamp(seconds))
            .await
        {
            Ok(true) => {
                let c = &self.status_config;
                let request = StatusRequest {
                    local_query_id: query_id.clone(),
                    lookup: StatusLookup::Receipt(Id::new(receipt.as_str()).expect("receipt")),
                    initiator: c.initiator.clone(),
                    security_credential: c.security_credential.clone(),
                    party_a: c.shortcode.clone(),
                    identifier_type: StatusIdentifierType::Shortcode,
                    result_url: c.result_url.clone(),
                    timeout_url: c.timeout_url.clone(),
                    remarks: Secret::new("inbound reconciliation"),
                    occasion: Secret::new("inbound"),
                };
                if let Err(e) = self.status.request(&request, &self.repository).await {
                    return if e.kind == ErrorKind::Throttled {
                        QueryState::Throttled
                    } else {
                        QueryState::Unresolved
                    };
                }
            }
            Ok(false) => {}
            Err(_) => return QueryState::Unresolved,
        }
        let Ok(results) = self.repository.status_results(query_id).await else {
            return QueryState::Unresolved;
        };
        if results.is_empty() {
            return QueryState::Unresolved;
        }
        for body in results {
            let Ok(r) = StatusResult::parse(body.expose()) else {
                return QueryState::Unresolved;
            };
            if r.observed_amount.is_some_and(|a| Some(a) != amount)
                || r.result_code.as_str() != "0"
                || (r.receipt.as_ref() != Some(receipt)
                    && r.transaction_id.as_ref().map(Id::as_str) != Some(receipt.as_str()))
            {
                return QueryState::Unresolved;
            }
        }
        QueryState::Success
    }
}
impl StatusPersistence for Repository {
    async fn prepare(&self, record: StatusPrepared<'_>) -> crate::transport::Result<()> {
        let StatusLookup::Receipt(receipt) = record.lookup else {
            return Err(ClientError::new(ErrorKind::CapabilityDisabled));
        };
        let r = ReceiptId::new(receipt.as_str()).map_err(|_| ClientError::new(ErrorKind::Input))?;
        let seconds = record
            .created_at
            .timestamp()
            .try_into()
            .map_err(|_| ClientError::new(ErrorKind::Input))?;
        self.status_prepared(record.local_query_id, &r, Timestamp(seconds))
            .await
            .map_err(|_| ClientError::new(ErrorKind::Persistence))?;
        Ok(())
    }
    async fn acknowledge(
        &self,
        id: &Id,
        ack: &StatusAcknowledgment,
    ) -> crate::transport::Result<()> {
        self.status_ack(id, &ack.originator_conversation_id, &ack.conversation_id)
            .await
            .map_err(|_| ClientError::new(ErrorKind::Persistence))
    }
}

/// Consumer-owned cursor. Advance only after durably handling the returned event.
/// Reopening at the previous cursor redelivers safely; sequence zero replays all history.
pub struct IncomingEvents {
    repository: Repository,
    cursor: i64,
    offered: Option<i64>,
}
impl IncomingEvents {
    pub fn resume(repository: Repository, cursor: i64) -> Result<Self> {
        if cursor < 0 {
            return Err(Error::InvalidIdentity.into());
        }
        Ok(Self {
            repository,
            cursor,
            offered: None,
        })
    }
    pub fn acknowledge(&mut self, sequence: i64) -> Result<()> {
        if self.offered != Some(sequence) || sequence <= self.cursor {
            return Err(Error::InvalidTransition.into());
        }
        self.cursor = sequence;
        self.offered = None;
        Ok(())
    }
    pub async fn next(&mut self) -> Result<(i64, SettlementEvent)> {
        loop {
            if let Some(event) = self.repository.events_after(self.cursor, 1).await?.pop() {
                self.offered = Some(event.0);
                return Ok(event);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

//! Untrusted callback admission. No settlement or receipt-allocation authority lives here.
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    body::to_bytes,
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use ipnet::IpNet;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{
    Timestamp,
    collection::{StatusResult, StatusTimeout, parse_kes},
    crypto::Secret,
    identity::{Id, ReceiptId},
    payout::PayoutResult,
    repository::Repository,
    transport::{scalar, strict_json},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Endpoint {
    Stk,
    C2bValidation,
    C2bConfirmation,
    B2c,
    B2b,
    B2cTimeout,
    B2bTimeout,
    StatusResult,
    StatusTimeout,
    BalanceResult,
    BalanceTimeout,
}
impl Endpoint {
    pub const ALL: [Self; 11] = [
        Self::Stk,
        Self::C2bValidation,
        Self::C2bConfirmation,
        Self::B2c,
        Self::B2b,
        Self::B2cTimeout,
        Self::B2bTimeout,
        Self::StatusResult,
        Self::StatusTimeout,
        Self::BalanceResult,
        Self::BalanceTimeout,
    ];
    pub fn path(self) -> &'static str {
        match self {
            Self::Stk => "/callbacks/stk/{secret}",
            Self::C2bValidation => "/callbacks/c2b/validation/{secret}",
            Self::C2bConfirmation => "/callbacks/c2b/confirmation/{secret}",
            Self::B2c => "/callbacks/b2c/{secret}",
            Self::B2b => "/callbacks/b2b/{secret}",
            Self::B2cTimeout => "/callbacks/b2c/timeout/{secret}",
            Self::B2bTimeout => "/callbacks/b2b/timeout/{secret}",
            Self::StatusResult => "/callbacks/status/result/{secret}",
            Self::StatusTimeout => "/callbacks/status/timeout/{secret}",
            Self::BalanceResult => "/callbacks/balance/result/{secret}",
            Self::BalanceTimeout => "/callbacks/balance/timeout/{secret}",
        }
    }
}

/// Supply independent CSPRNG-generated 32-byte secrets encoded as 64 lowercase hex digits.
/// Format validation cannot prove entropy. This type deliberately has no Debug implementation.
pub struct Policy {
    secrets: BTreeMap<Endpoint, [u8; 32]>,
    shortcode: Id,
    body_limit: usize,
    deadline: Duration,
    c2b_validation_verified: bool,
    allow_held_topups: bool,
    direct_peers: Option<Vec<IpNet>>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct InvalidPolicy;
impl Policy {
    pub fn new(shortcode: Id, secrets: BTreeMap<Endpoint, Secret>) -> Result<Self, InvalidPolicy> {
        let mut digests = BTreeMap::new();
        for endpoint in Endpoint::ALL {
            let secret = secrets.get(&endpoint).ok_or(InvalidPolicy)?.expose();
            if secret.len() != 64
                || !secret
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(InvalidPolicy);
            }
            let digest: [u8; 32] = Sha256::digest(secret.as_bytes()).into();
            if digests.values().any(|existing| existing == &digest) {
                return Err(InvalidPolicy);
            }
            digests.insert(endpoint, digest);
        }
        Ok(Self {
            secrets: digests,
            shortcode,
            body_limit: 32 * 1024,
            deadline: Duration::from_millis(500),
            c2b_validation_verified: false,
            allow_held_topups: false,
            direct_peers: None,
        })
    }
    pub fn with_limits(
        mut self,
        body_bytes: usize,
        deadline: Duration,
    ) -> Result<Self, InvalidPolicy> {
        if body_bytes == 0
            || body_bytes > 1024 * 1024
            || deadline.is_zero()
            || deadline > Duration::from_secs(2)
        {
            return Err(InvalidPolicy);
        }
        self.body_limit = body_bytes;
        self.deadline = deadline;
        Ok(self)
    }
    /// Operator attestation: activation, codes and timeout/default behavior verified on this shortcode.
    pub fn enable_verified_c2b_validation(mut self, allow_held_topups: bool) -> Self {
        self.c2b_validation_verified = true;
        self.allow_held_topups = allow_held_topups;
        self
    }
    /// Applies to the TCP peer only. Forwarding headers are rejected; proxy chains are unsupported.
    pub fn restrict_direct_peers(mut self, networks: Vec<IpNet>) -> Self {
        self.direct_peers = Some(networks);
        self
    }
}
struct Ingress {
    repository: Repository,
    policy: Policy,
}

pub fn router(repository: Repository, policy: Policy) -> Router {
    let state = Arc::new(Ingress { repository, policy });
    let mut router = Router::new();
    for endpoint in Endpoint::ALL {
        router = router.route(
            endpoint.path(),
            post(
                move |State(state): State<Arc<Ingress>>, request: Request| async move {
                    ingress(state, endpoint, request).await
                },
            ),
        );
    }
    router.with_state(state)
}

fn error(status: StatusCode) -> Response {
    (status, Json(json!({"error":"callback rejected"}))).into_response()
}
fn validation(code: &'static str) -> Response {
    let description = match code {
        "0" => "Accepted",
        "C2B00012" => "Invalid Account Number",
        "C2B00013" => "Invalid Amount",
        _ => "Rejected",
    };
    Json(json!({"ResultCode":code,"ResultDesc":description})).into_response()
}
async fn ingress(state: Arc<Ingress>, endpoint: Endpoint, request: Request) -> Response {
    let secret = request.uri().path().rsplit('/').next().unwrap_or("");
    let supplied: [u8; 32] = Sha256::digest(secret.as_bytes()).into();
    if !bool::from(state.policy.secrets[&endpoint].ct_eq(&supplied)) {
        return error(StatusCode::NOT_FOUND);
    }
    // Never derive a trusted sender from an attacker-controlled forwarding header.
    if ["forwarded", "x-forwarded-for", "x-real-ip"]
        .iter()
        .any(|key| request.headers().contains_key(*key))
    {
        return error(StatusCode::FORBIDDEN);
    }
    if let Some(networks) = &state.policy.direct_peers {
        let peer = request.extensions().get::<ConnectInfo<SocketAddr>>();
        if !peer.is_some_and(|peer| {
            networks
                .iter()
                .any(|network| network.contains(&peer.0.ip()))
        }) {
            return error(StatusCode::FORBIDDEN);
        }
    }
    if endpoint == Endpoint::C2bValidation && !state.policy.c2b_validation_verified {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    }
    match tokio::time::timeout(state.policy.deadline, receive(&state, endpoint, request)).await {
        Ok(response) => response,
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE),
    }
}
async fn receive(state: &Ingress, endpoint: Endpoint, request: Request) -> Response {
    let bytes = match to_bytes(request.into_body(), state.policy.body_limit).await {
        Ok(bytes) => bytes,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE),
    };
    let body = match std::str::from_utf8(&bytes) {
        Ok(body) => body,
        Err(_) => return error(StatusCode::BAD_REQUEST),
    };
    let value = match strict_json(body) {
        Ok(value) if value.is_object() => value,
        _ => return error(StatusCode::BAD_REQUEST),
    };
    let now = Timestamp(chrono::Utc::now().timestamp().max(0) as u64);
    if endpoint == Endpoint::C2bValidation {
        let payload = match C2b::parse(&value) {
            Some(payload) => payload,
            None => return error(StatusCode::BAD_REQUEST),
        };
        if payload.shortcode != state.policy.shortcode {
            return validation("C2B00012");
        }
        return match state
            .repository
            .c2b_expected(
                &payload.account,
                &payload.shortcode,
                now,
                state.policy.allow_held_topups,
            )
            .await
        {
            Ok(None) => validation("C2B00012"),
            Ok(Some(expected)) if Some(expected) == payload.amount => validation("0"),
            Ok(Some(_)) => validation("C2B00013"),
            Err(_) => error(StatusCode::SERVICE_UNAVAILABLE),
        };
    }
    let quote = match bind(state, endpoint, &value, body).await {
        Ok(quote) => quote,
        Err(status) => return error(status),
    };
    match state
        .repository
        .accept_callback(endpoint.path(), quote.as_ref(), body, now)
        .await
    {
        Ok(()) => validation("0"),
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE),
    }
}
fn field(value: &Value, key: &str) -> Option<String> {
    scalar(value.get(key)?).filter(|s| !s.is_empty() && s.len() <= 256)
}
struct C2b {
    account: String,
    shortcode: Id,
    amount: Option<crate::amount::Amount>,
}
impl C2b {
    fn parse(value: &Value) -> Option<Self> {
        ReceiptId::new(&field(value, "TransID")?).ok()?;
        field(value, "TransactionType")?;
        provider_time(&field(value, "TransTime")?)?;
        field(value, "MSISDN")?; // Accept masked/hashed forms; this field is never a binding key.
        Some(Self {
            account: field(value, "BillRefNumber")?,
            shortcode: Id::new(field(value, "BusinessShortCode")?).ok()?,
            amount: parse_kes(value.get("TransAmount")?).ok(),
        })
    }
}
struct StkCallback {
    checkout: String,
    merchant: String,
}
impl StkCallback {
    fn parse(value: &Value) -> Option<Self> {
        let callback = value.get("Body")?.get("stkCallback")?;
        let checkout = field(callback, "CheckoutRequestID")?;
        let merchant = field(callback, "MerchantRequestID")?;
        let code = field(callback, "ResultCode")?.parse::<u32>().ok()?;
        field(callback, "ResultDesc")?;
        if code == 0 {
            let items = callback.pointer("/CallbackMetadata/Item")?.as_array()?;
            let mut metadata = BTreeMap::new();
            for item in items {
                let name = field(item, "Name")?;
                if metadata.insert(name, item.get("Value")).is_some() {
                    return None;
                }
            }
            let get = |key| metadata.get(key).copied().flatten();
            parse_kes(get("Amount")?).ok()?;
            ReceiptId::new(&scalar(get("MpesaReceiptNumber")?)?).ok()?;
            provider_time(&scalar(get("TransactionDate")?)?)?;
            scalar(get("PhoneNumber")?)?;
        }
        Some(Self { checkout, merchant })
    }
}
fn provider_time(text: &str) -> Option<()> {
    if text.len() != 14 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    chrono::NaiveDateTime::parse_from_str(text, "%Y%m%d%H%M%S")
        .ok()
        .map(|_| ())
}
async fn bind(
    state: &Ingress,
    endpoint: Endpoint,
    value: &Value,
    body: &str,
) -> Result<Option<Id>, StatusCode> {
    let bad = StatusCode::BAD_REQUEST;
    let unavailable = StatusCode::SERVICE_UNAVAILABLE;
    match endpoint {
        Endpoint::Stk => {
            let callback = StkCallback::parse(value).ok_or(bad)?;
            let checkout = callback.checkout;
            let merchant = callback.merchant;
            let quote = state
                .repository
                .callback_quote(&checkout, true)
                .await
                .map_err(|_| unavailable)?
                .ok_or(bad)?;
            if quote.shortcode() != &state.policy.shortcode
                || !quote.attempts().iter().any(|a| {
                    a.checkout_request()
                        .is_some_and(|id| id.as_str() == checkout)
                        && a.merchant_request()
                            .is_some_and(|id| id.as_str() == merchant)
                })
            {
                return Err(bad);
            }
            Ok(Some(quote.id().clone()))
        }
        Endpoint::C2bConfirmation => {
            let payload = C2b::parse(value).ok_or(bad)?;
            payload.amount.ok_or(bad)?;
            if payload.shortcode != state.policy.shortcode {
                return Err(bad);
            }
            let quote = state
                .repository
                .callback_quote(&payload.account, false)
                .await
                .map_err(|_| unavailable)?;
            if quote
                .as_ref()
                .is_some_and(|q| q.shortcode() != &payload.shortcode)
            {
                return Err(bad);
            }
            // Unmatched/expired/mismatched amounts remain durable observations for exception review.
            Ok(quote.map(|q| q.id().clone()))
        }
        Endpoint::B2c | Endpoint::B2b => {
            PayoutResult::parse(body).map_err(|_| bad)?;
            Ok(None)
        }
        Endpoint::StatusResult | Endpoint::BalanceResult => {
            StatusResult::parse(body).map_err(|_| bad)?;
            Ok(None)
        }
        Endpoint::B2cTimeout
        | Endpoint::B2bTimeout
        | Endpoint::StatusTimeout
        | Endpoint::BalanceTimeout => {
            StatusTimeout::parse(body).map_err(|_| bad)?;
            Ok(None)
        }
        Endpoint::C2bValidation => Err(bad),
    }
}

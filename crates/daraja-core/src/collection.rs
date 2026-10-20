//! Collection operations and asynchronous status observations, never settlement evidence.
use crate::{
    amount::{Amount, Unit},
    auth::{CollectionApp, DisbursementApp, OAuthActor},
    crypto::{Clock, Secret, stk_password},
    identity::{Id, ReceiptId},
    transport::{
        ClientError, ErrorKind, HttpRequest, HttpResponse, HttpTransport, Outcome, Result,
        validate_base,
    },
};
use reqwest::{Url, header::HeaderValue};
use serde_json::{Value, json};
use std::{collections::BTreeMap, future::Future, time::Duration};

pub struct StkConfig {
    pub shortcode: Id,
    pub passkey: Secret,
}
#[derive(Debug)]
pub struct StkSubmission {
    pub amount: Amount,
    pub phone: Secret,
    pub callback_url: Secret,
    pub account_reference: Id,
    pub description: Secret,
    pub transaction_type: StkTransactionType,
}
#[derive(Clone, Copy, Debug)]
pub enum StkTransactionType {
    CustomerPayBillOnline,
    CustomerBuyGoodsOnline,
}
impl StkTransactionType {
    fn wire(self) -> &'static str {
        match self {
            Self::CustomerPayBillOnline => "CustomerPayBillOnline",
            Self::CustomerBuyGoodsOnline => "CustomerBuyGoodsOnline",
        }
    }
}
#[derive(Debug)]
pub struct StkAcknowledgment {
    pub outcome: Outcome,
    pub merchant_request_id: Id,
    pub checkout_request_id: Id,
}
#[derive(Debug)]
pub struct StkQueryObservation {
    pub outcome: Outcome,
    pub merchant_request_id: Id,
    pub checkout_request_id: Id,
    pub result_code: Id,
}
#[derive(Clone, Copy, Debug, Default)]
pub enum C2bResponsePolicy {
    Completed,
    #[default]
    Cancelled,
}
#[derive(Debug)]
pub struct C2bRegistration {
    pub shortcode: Id,
    pub confirmation_url: Secret,
    pub validation_url: Secret,
    pub response_policy: C2bResponsePolicy,
}

/// Applies only to read-only STK queries. Submissions and async status requests never use it.
#[derive(Clone, Copy, Debug)]
pub struct QueryRetryPolicy {
    max_retries: u8,
    initial: Duration,
    cap: Duration,
}
impl QueryRetryPolicy {
    pub fn new(max_retries: u8, initial: Duration, cap: Duration) -> Result<Self> {
        if max_retries > 5 || initial.is_zero() || initial > cap || cap > Duration::from_secs(60) {
            return Err(ClientError::new(ErrorKind::Config));
        }
        Ok(Self {
            max_retries,
            initial,
            cap,
        })
    }
    /// Full jitter in [0, capped exponential delay], with injected entropy for reproducible tests.
    pub fn delay(&self, retry: u8, entropy: u64) -> Duration {
        let ceiling = self
            .initial
            .saturating_mul(1u32 << retry.min(31))
            .min(self.cap);
        let nanos = ceiling.as_nanos() as u64;
        Duration::from_nanos(entropy % (nanos + 1))
    }
}
pub trait QueryRuntime: Send + Sync {
    fn entropy(&self) -> u64;
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}
pub struct TokioQueryRuntime;
impl QueryRuntime for TokioQueryRuntime {
    fn entropy(&self) -> u64 {
        use rsa::rand_core::RngCore;
        rsa::rand_core::OsRng.next_u64()
    }
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

pub struct CollectionClient<T, C> {
    transport: T,
    base: Url,
    app: CollectionApp,
    clock: C,
    stk: StkConfig,
}
impl<T: HttpTransport, C: Clock> CollectionClient<T, C> {
    pub fn new(
        transport: T,
        base: Url,
        app: CollectionApp,
        clock: C,
        stk: StkConfig,
    ) -> Result<Self> {
        validate_base(&base)?;
        if !stk.shortcode.as_str().bytes().all(|b| b.is_ascii_digit())
            || stk.passkey.expose().is_empty()
        {
            return Err(ClientError::new(ErrorKind::Config));
        }
        Ok(Self {
            transport,
            base,
            app,
            clock,
            stk,
        })
    }
    fn stk_body(&self) -> Value {
        let password = stk_password(self.stk.shortcode.as_str(), &self.stk.passkey, &self.clock);
        json!({"BusinessShortCode":self.stk.shortcode.as_str(), "Password":password.password.expose(), "Timestamp":password.timestamp})
    }
    async fn post(&self, path: &str, body: Value) -> Result<(Value, Outcome)> {
        authorized_send(&self.app, &self.transport, &self.base, path, body).await
    }
    pub async fn submit_stk(&self, request: &StkSubmission) -> Result<StkAcknowledgment> {
        let amount = request
            .amount
            .whole_kes()
            .map_err(|_| ClientError::new(ErrorKind::Input))?;
        if amount == 0
            || request.phone.expose().len() != 12
            || !request.phone.expose().starts_with("254")
            || !request.phone.expose().bytes().all(|b| b.is_ascii_digit())
        {
            return Err(ClientError::new(ErrorKind::Input));
        }
        callback_url(&request.callback_url)?;
        let mut body = self.stk_body();
        let object = body.as_object_mut().expect("object");
        object.extend(json!({"TransactionType":request.transaction_type.wire(), "Amount":amount,
            "PartyA":request.phone.expose(), "PartyB":self.stk.shortcode.as_str(), "PhoneNumber":request.phone.expose(),
            "CallBackURL":request.callback_url.expose(), "AccountReference":request.account_reference.as_str(),
            "TransactionDesc":request.description.expose()}).as_object().expect("object").clone());
        let (value, outcome) = self.post("/mpesa/stkpush/v1/processrequest", body).await?;
        Ok(StkAcknowledgment {
            merchant_request_id: required_id(&value, "MerchantRequestID", &outcome)?,
            checkout_request_id: required_id(&value, "CheckoutRequestID", &outcome)?,
            outcome,
        })
    }
    pub async fn query_stk(&self, checkout: &Id) -> Result<StkQueryObservation> {
        let mut body = self.stk_body();
        body["CheckoutRequestID"] = json!(checkout.as_str());
        let (value, outcome) = self.post("/mpesa/stkpushquery/v1/query", body).await?;
        let returned = required_id(&value, "CheckoutRequestID", &outcome)?;
        if returned != *checkout {
            return Err(invalid(&outcome));
        }
        Ok(StkQueryObservation {
            merchant_request_id: required_id(&value, "MerchantRequestID", &outcome)?,
            checkout_request_id: returned,
            result_code: required_id(&value, "ResultCode", &outcome)?,
            outcome,
        })
    }
    pub async fn query_stk_with_retry(
        &self,
        checkout: &Id,
        policy: QueryRetryPolicy,
        runtime: &impl QueryRuntime,
    ) -> Result<StkQueryObservation> {
        let mut retries = 0;
        loop {
            match self.query_stk(checkout).await {
                Err(error)
                    if error.kind == ErrorKind::Throttled && retries < policy.max_retries =>
                {
                    runtime
                        .sleep(policy.delay(retries, runtime.entropy()))
                        .await;
                    retries += 1;
                }
                result => return result,
            }
        }
    }
    /// Explicit operator action. Registration does not activate provider-side validation.
    pub async fn register_c2b(&self, registration: &C2bRegistration) -> Result<Outcome> {
        callback_url(&registration.confirmation_url)?;
        callback_url(&registration.validation_url)?;
        let response_type = match registration.response_policy {
            C2bResponsePolicy::Completed => "Completed",
            C2bResponsePolicy::Cancelled => "Cancelled",
        };
        let (_, outcome) = self.post("/mpesa/c2b/v1/registerurl", json!({"ShortCode":registration.shortcode.as_str(),
            "ResponseType":response_type, "ConfirmationURL":registration.confirmation_url.expose(), "ValidationURL":registration.validation_url.expose()})).await?;
        Ok(outcome)
    }
}
fn auth_error(error: crate::auth::AuthError) -> ClientError {
    ClientError::new(ErrorKind::Auth(error))
}
fn callback_url(secret: &Secret) -> Result<()> {
    let url = Url::parse(secret.expose()).map_err(|_| ClientError::new(ErrorKind::Input))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(ClientError::new(ErrorKind::Input));
    }
    Ok(())
}

/// The app must match the account being queried; no fallback between credential actors.
pub enum StatusCredentials {
    Collection(CollectionApp),
    Disbursement(DisbursementApp),
}
impl std::fmt::Debug for StatusCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Collection(_) => "Collection",
            Self::Disbursement(_) => "Disbursement",
        })
    }
}
#[derive(Debug)]
pub enum StatusLookup {
    Receipt(Id),
    OriginalConversation(Id),
}
#[derive(Debug)]
pub struct StatusRequest {
    pub local_query_id: Id,
    pub lookup: StatusLookup,
    pub initiator: Secret,
    pub security_credential: Secret,
    pub party_a: Id,
    pub identifier_type: StatusIdentifierType,
    pub result_url: Secret,
    pub timeout_url: Secret,
    pub remarks: Secret,
    pub occasion: Secret,
}
#[derive(Clone, Copy, Debug)]
pub enum StatusIdentifierType {
    Msisdn,
    Till,
    Shortcode,
}
impl StatusIdentifierType {
    fn wire(self) -> u8 {
        match self {
            Self::Msisdn => 1,
            Self::Till => 2,
            Self::Shortcode => 4,
        }
    }
}
#[derive(Debug)]
pub struct StatusPrepared<'a> {
    pub local_query_id: &'a Id,
    pub lookup: &'a StatusLookup,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
#[derive(Debug)]
pub struct StatusAcknowledgment {
    pub outcome: Outcome,
    pub originator_conversation_id: Id,
    pub conversation_id: Id,
}
/// Commit prepare before I/O; bind acknowledgment atomically, preserving the prepared row
/// on ambiguity. Implementations must tolerate early callbacks and reject conflicting IDs.
pub trait StatusPersistence: Send + Sync {
    fn prepare(&self, record: StatusPrepared<'_>) -> impl Future<Output = Result<()>> + Send;
    fn acknowledge(
        &self,
        local_query_id: &Id,
        acknowledgment: &StatusAcknowledgment,
    ) -> impl Future<Output = Result<()>> + Send;
}
pub struct TransactionStatusClient<T, C> {
    transport: T,
    base: Url,
    credentials: StatusCredentials,
    clock: C,
    original_conversation_enabled: bool,
}
impl<T: HttpTransport, C: Clock> TransactionStatusClient<T, C> {
    pub fn new(transport: T, base: Url, credentials: StatusCredentials, clock: C) -> Result<Self> {
        validate_base(&base)?;
        Ok(Self {
            transport,
            base,
            credentials,
            clock,
            original_conversation_enabled: false,
        })
    }
    /// Requires independently recorded STATUS-ID evidence for this endpoint/environment.
    pub fn enable_original_conversation_lookup(&mut self) {
        self.original_conversation_enabled = true;
    }
    pub async fn request(
        &self,
        request: &StatusRequest,
        persistence: &impl StatusPersistence,
    ) -> Result<StatusAcknowledgment> {
        if matches!(request.lookup, StatusLookup::OriginalConversation(_))
            && !self.original_conversation_enabled
        {
            return Err(ClientError::new(ErrorKind::CapabilityDisabled));
        }
        callback_url(&request.result_url)?;
        callback_url(&request.timeout_url)?;
        let mut body = json!({"Initiator":request.initiator.expose(), "SecurityCredential":request.security_credential.expose(),
            "CommandID":"TransactionStatusQuery", "PartyA":request.party_a.as_str(), "IdentifierType":request.identifier_type.wire(),
            "ResultURL":request.result_url.expose(), "QueueTimeOutURL":request.timeout_url.expose(),
            "Remarks":request.remarks.expose(), "Occasion":request.occasion.expose()});
        match &request.lookup {
            StatusLookup::Receipt(id) => body["TransactionID"] = json!(id.as_str()),
            StatusLookup::OriginalConversation(id) => {
                body["OriginalConversationID"] = json!(id.as_str())
            }
        }
        persistence
            .prepare(StatusPrepared {
                local_query_id: &request.local_query_id,
                lookup: &request.lookup,
                created_at: self.clock.now(),
            })
            .await?;
        let path = "/mpesa/transactionstatus/v1/query";
        let (value, outcome) = match &self.credentials {
            StatusCredentials::Collection(app) => {
                authorized_send(app, &self.transport, &self.base, path, body).await
            }
            StatusCredentials::Disbursement(app) => {
                authorized_send(app, &self.transport, &self.base, path, body).await
            }
        }?;
        let acknowledgment = StatusAcknowledgment {
            originator_conversation_id: required_id(&value, "OriginatorConversationID", &outcome)?,
            conversation_id: required_id(&value, "ConversationID", &outcome)?,
            outcome,
        };
        persistence
            .acknowledge(&request.local_query_id, &acknowledgment)
            .await
            .map_err(|_| ClientError {
                kind: ErrorKind::Persistence,
                outcome: Box::new(acknowledgment.outcome.clone()),
            })?;
        Ok(acknowledgment)
    }
}

async fn authorized_send<App>(
    app: &OAuthActor<App>,
    transport: &impl HttpTransport,
    base: &Url,
    path: &str,
    body: Value,
) -> Result<(Value, Outcome)> {
    let mut attempt = app.authorize().await.map_err(auth_error)?;
    let header = attempt
        .token()
        .expect("new authorization attempt")
        .authorization()
        .map_err(auth_error)?;
    let result = send(transport, base, path, header, body).await;
    if result
        .as_ref()
        .is_err_and(|error| error.outcome.http_status == Some(401))
    {
        // Refresh the rejected generation for future calls, preserving this request's outcome.
        let _ = attempt.refresh_after_unauthorized().await;
    }
    result
}

async fn send(
    transport: &impl HttpTransport,
    base: &Url,
    path: &str,
    authorization: HeaderValue,
    body: Value,
) -> Result<(Value, Outcome)> {
    let response = transport
        .post(HttpRequest {
            url: base
                .join(path)
                .map_err(|_| ClientError::new(ErrorKind::Config))?,
            authorization,
            body: Secret::new(body.to_string()),
        })
        .await?;
    decode_response(response)
}
fn decode_response(response: HttpResponse) -> Result<(Value, Outcome)> {
    let mut outcome = Outcome {
        http_status: Some(response.status),
        request_id: response.request_id,
        ..Outcome::default()
    };
    let value = strict_json(response.body.expose()).map_err(|_| ClientError {
        kind: if (200..300).contains(&response.status) {
            ErrorKind::InvalidResponse
        } else {
            ErrorKind::Http
        },
        outcome: Box::new(outcome.clone()),
    })?;
    outcome.request_id = optional_id(&value, "requestId").or(outcome.request_id);
    outcome.provider_code =
        optional_id(&value, "errorCode").or_else(|| optional_id(&value, "ResponseCode"));
    outcome.originator_conversation_id = optional_id(&value, "OriginatorConversationID");
    outcome.conversation_id = optional_id(&value, "ConversationID");
    outcome.merchant_request_id = optional_id(&value, "MerchantRequestID");
    outcome.checkout_request_id = optional_id(&value, "CheckoutRequestID");
    let kind = if outcome
        .provider_code
        .as_ref()
        .is_some_and(|c| c.as_str() == "500.001.1001")
    {
        Some(ErrorKind::Throttled)
    } else if !(200..300).contains(&response.status) {
        Some(ErrorKind::Http)
    } else if value.get("errorCode").is_some() {
        Some(ErrorKind::Provider)
    } else {
        match outcome.provider_code.as_ref().map(Id::as_str) {
            Some("0") => None,
            Some(_) => Some(ErrorKind::Provider),
            None => Some(ErrorKind::InvalidResponse),
        }
    };
    if let Some(kind) = kind {
        return Err(ClientError {
            kind,
            outcome: Box::new(outcome),
        });
    }
    Ok((value, outcome))
}
fn invalid(outcome: &Outcome) -> ClientError {
    ClientError {
        kind: ErrorKind::InvalidResponse,
        outcome: Box::new(outcome.clone()),
    }
}
fn optional_id(value: &Value, field: &str) -> Option<Id> {
    scalar(value.get(field)?).and_then(|s| Id::new(s).ok())
}
fn required_id(value: &Value, field: &str, outcome: &Outcome) -> Result<Id> {
    optional_id(value, field).ok_or_else(|| invalid(outcome))
}
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) if n.is_i64() || n.is_u64() => Some(n.to_string()),
        _ => None,
    }
}

pub struct StatusResult {
    pub originator_conversation_id: Id,
    pub conversation_id: Id,
    pub result_code: Id,
    pub transaction_id: Option<Id>,
    pub parameters: BTreeMap<String, Secret>,
    pub observed_amount: Option<Amount>,
    pub receipt: Option<ReceiptId>,
}
impl std::fmt::Debug for StatusResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StatusResult([REDACTED])")
    }
}
impl StatusResult {
    /// Untrusted observation only. Duplicate object/parameter keys fail closed.
    pub fn parse(body: &str) -> Result<Self> {
        let value = strict_json(body).map_err(|_| ClientError::new(ErrorKind::InvalidResponse))?;
        let value = value
            .get("Result")
            .ok_or_else(|| ClientError::new(ErrorKind::InvalidResponse))?;
        let outcome = Outcome::default();
        let mut parameters = BTreeMap::new();
        if let Some(container) = value.get("ResultParameters") {
            let items = container
                .get("ResultParameter")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid(&outcome))?;
            for item in items {
                let key = item
                    .get("Key")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid(&outcome))?;
                let data = item
                    .get("Value")
                    .and_then(scalar)
                    .ok_or_else(|| invalid(&outcome))?;
                if parameters
                    .insert(key.to_owned(), Secret::new(data))
                    .is_some()
                {
                    return Err(invalid(&outcome));
                }
            }
        }
        let transaction_id = if value.get("TransactionID").is_some() {
            Some(required_id(value, "TransactionID", &outcome)?)
        } else {
            None
        };
        let observed_amount = parameters
            .get("Amount")
            .map(|amount| parse_kes(&json!(amount.expose())).map_err(|_| invalid(&outcome)))
            .transpose()?;
        let receipt = parameters
            .get("ReceiptNo")
            .map(|receipt| ReceiptId::new(receipt.expose()).map_err(|_| invalid(&outcome)))
            .transpose()?;
        if let (Some(receipt), Some(transaction)) = (&receipt, &transaction_id)
            && ReceiptId::new(transaction.as_str()).ok().as_ref() != Some(receipt)
        {
            return Err(invalid(&outcome));
        }
        Ok(Self {
            observed_amount,
            receipt,
            originator_conversation_id: required_id(value, "OriginatorConversationID", &outcome)?,
            conversation_id: required_id(value, "ConversationID", &outcome)?,
            result_code: required_id(value, "ResultCode", &outcome)?,
            transaction_id,
            parameters,
        })
    }
}
#[derive(Debug)]
pub struct StatusTimeout {
    pub originator_conversation_id: Id,
    pub conversation_id: Id,
    pub result_code: Id,
}
impl StatusTimeout {
    pub fn parse(body: &str) -> Result<Self> {
        let result = StatusResult::parse(body)?;
        Ok(Self {
            originator_conversation_id: result.originator_conversation_id,
            conversation_id: result.conversation_id,
            result_code: result.result_code,
        })
    }
}
/// Exact decimal KES strings and integer JSON numbers only. Floats/exponents are rejected.
pub fn parse_kes(value: &Value) -> Result<Amount> {
    let text = scalar(value).ok_or_else(|| ClientError::new(ErrorKind::Input))?;
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 2
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (text.contains('.') && fraction.is_empty())
    {
        return Err(ClientError::new(ErrorKind::Input));
    }
    let whole = whole.parse::<u64>().ok().and_then(|n| n.checked_mul(100));
    let fraction = match fraction.len() {
        0 => Some(0),
        1 => fraction.parse::<u64>().ok().and_then(|n| n.checked_mul(10)),
        _ => fraction.parse::<u64>().ok(),
    };
    let minor = whole
        .and_then(|w| fraction.and_then(|f| w.checked_add(f)))
        .filter(|n| *n > 0)
        .ok_or_else(|| ClientError::new(ErrorKind::Input))?;
    Ok(Amount::new(minor, Unit::KesMinor))
}

fn strict_json(text: &str) -> std::result::Result<Value, serde_json::Error> {
    use serde::Deserialize;
    if text.len() > 64 * 1024 {
        return Err(<serde_json::Error as serde::de::Error>::custom(
            "body too large",
        ));
    }
    struct Unique(Value);
    impl<'de> Deserialize<'de> for Unique {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Unique;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("JSON without duplicate keys")
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> std::result::Result<Unique, A::Error> {
                    let mut object = serde_json::Map::new();
                    while let Some((key, Unique(value))) = map.next_entry::<String, Unique>()? {
                        if object.insert(key, value).is_some() {
                            return Err(serde::de::Error::custom("duplicate key"));
                        }
                    }
                    Ok(Unique(Value::Object(object)))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> std::result::Result<Unique, A::Error> {
                    let mut items = Vec::new();
                    while let Some(Unique(item)) = seq.next_element()? {
                        items.push(item);
                    }
                    Ok(Unique(Value::Array(items)))
                }
                fn visit_str<E: serde::de::Error>(self, s: &str) -> std::result::Result<Unique, E> {
                    Ok(Unique(Value::String(s.to_owned())))
                }
                fn visit_bool<E: serde::de::Error>(
                    self,
                    b: bool,
                ) -> std::result::Result<Unique, E> {
                    Ok(Unique(Value::Bool(b)))
                }
                fn visit_i64<E: serde::de::Error>(self, n: i64) -> std::result::Result<Unique, E> {
                    Ok(Unique(json!(n)))
                }
                fn visit_u64<E: serde::de::Error>(self, n: u64) -> std::result::Result<Unique, E> {
                    Ok(Unique(json!(n)))
                }
                fn visit_f64<E: serde::de::Error>(self, _: f64) -> std::result::Result<Unique, E> {
                    Err(E::custom("floating point unsupported"))
                }
                fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
            }
            d.deserialize_any(Visitor)
        }
    }
    serde_json::from_str::<Unique>(text).map(|v| v.0)
}

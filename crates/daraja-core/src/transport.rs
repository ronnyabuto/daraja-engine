//! Bounded HTTP boundary. Wire bodies and nested transport errors are never logged.
use crate::{
    auth::{AuthError, OAuthActor},
    crypto::Secret,
    identity::Id,
};
use reqwest::{Url, header::HeaderValue};
use serde_json::{Value, json};
use std::{future::Future, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Connect,
    Request,
    Body,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Config,
    Input,
    Auth(AuthError),
    Timeout(Phase),
    Transport(Phase),
    Http,
    Provider,
    Throttled,
    InvalidResponse,
    BodyLimit,
    Persistence,
    CapabilityDisabled,
}
#[derive(serde::Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub http_status: Option<u16>,
    pub request_id: Option<Id>,
    pub provider_code: Option<Id>,
    pub transaction_id: Option<Id>,
    pub originator_conversation_id: Option<Id>,
    pub conversation_id: Option<Id>,
    pub merchant_request_id: Option<Id>,
    pub checkout_request_id: Option<Id>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientError {
    pub kind: ErrorKind,
    pub outcome: Box<Outcome>,
}
impl ClientError {
    pub fn new(kind: ErrorKind) -> Self {
        Self {
            kind,
            outcome: Box::default(),
        }
    }
}
impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Daraja client error: {:?}", self.kind)
    }
}
impl std::error::Error for ClientError {}
pub type Result<T> = std::result::Result<T, ClientError>;

pub struct HttpRequest {
    pub url: Url,
    pub authorization: HeaderValue,
    pub body: Secret,
}
impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HttpRequest([REDACTED])")
    }
}
pub struct HttpResponse {
    pub status: u16,
    pub request_id: Option<Id>,
    pub body: Secret,
}
pub trait HttpTransport: Send + Sync {
    /// Implementations must enforce deadlines, body limits and never replay submissions.
    fn post(&self, request: HttpRequest) -> impl Future<Output = Result<HttpResponse>> + Send;
}
#[derive(Clone)]
pub struct ReqwestTransport(reqwest::Client);
impl ReqwestTransport {
    pub fn new(connect: Duration, request: Duration) -> Result<Self> {
        if connect.is_zero() || request.is_zero() || connect > request {
            return Err(ClientError::new(ErrorKind::Config));
        }
        reqwest::Client::builder()
            .connect_timeout(connect)
            .timeout(request)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .connection_verbose(false)
            .build()
            .map(Self)
            .map_err(|_| ClientError::new(ErrorKind::Config))
    }
}
impl HttpTransport for ReqwestTransport {
    async fn post(&self, request: HttpRequest) -> Result<HttpResponse> {
        let mut response = self
            .0
            .post(request.url)
            .header(reqwest::header::AUTHORIZATION, request.authorization)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(request.body.expose().to_owned())
            .send()
            .await
            .map_err(|e| transport_error(e, Phase::Request))?;
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| Id::new(v).ok());
        let outcome = Outcome {
            http_status: Some(status),
            request_id: request_id.clone(),
            ..Outcome::default()
        };
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| {
            let mut error = transport_error(e, Phase::Body);
            error.outcome = Box::new(outcome.clone());
            error
        })? {
            if bytes.len().saturating_add(chunk.len()) > 64 * 1024 {
                return Err(ClientError {
                    kind: ErrorKind::BodyLimit,
                    outcome: Box::new(outcome),
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = String::from_utf8(bytes).map_err(|_| ClientError {
            kind: ErrorKind::InvalidResponse,
            outcome: Box::new(outcome),
        })?;
        Ok(HttpResponse {
            status,
            request_id,
            body: Secret::new(body),
        })
    }
}
fn transport_error(e: reqwest::Error, phase: Phase) -> ClientError {
    let phase = if e.is_connect() {
        Phase::Connect
    } else {
        phase
    };
    ClientError::new(if e.is_timeout() {
        ErrorKind::Timeout(phase)
    } else {
        ErrorKind::Transport(phase)
    })
}

pub(crate) fn validate_base(base: &Url) -> Result<()> {
    let local = base.host_str().is_some_and(|h| {
        h.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    });
    if (base.scheme() != "https" && !(base.scheme() == "http" && local))
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
        || base.path() != "/"
    {
        return Err(ClientError::new(ErrorKind::Config));
    }
    Ok(())
}

fn auth_error(error: crate::auth::AuthError) -> ClientError {
    ClientError::new(ErrorKind::Auth(error))
}
pub(crate) fn callback_url(secret: &Secret) -> Result<()> {
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

pub(crate) async fn authorized_send<App>(
    app: &OAuthActor<App>,
    transport: &impl HttpTransport,
    base: &Url,
    path: &str,
    body: Value,
) -> Result<(Value, Outcome)> {
    authorized_send_checked(
        app,
        transport,
        base,
        path,
        Secret::new(body.to_string()),
        async { Ok(()) },
    )
    .await
}

pub(crate) async fn authorized_send_checked<App>(
    app: &OAuthActor<App>,
    transport: &impl HttpTransport,
    base: &Url,
    path: &str,
    body: Secret,
    before_send: impl Future<Output = Result<()>> + Send,
) -> Result<(Value, Outcome)> {
    let mut attempt = app.authorize().await.map_err(auth_error)?;
    let header = attempt
        .token()
        .expect("new authorization attempt")
        .authorization()
        .map_err(auth_error)?;
    before_send.await?;
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
    body: Secret,
) -> Result<(Value, Outcome)> {
    let response = transport
        .post(HttpRequest {
            url: base
                .join(path)
                .map_err(|_| ClientError::new(ErrorKind::Config))?,
            authorization,
            body,
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
    outcome.transaction_id = optional_id(&value, "TransactionID");
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
pub(crate) fn invalid(outcome: &Outcome) -> ClientError {
    ClientError {
        kind: ErrorKind::InvalidResponse,
        outcome: Box::new(outcome.clone()),
    }
}
fn optional_id(value: &Value, field: &str) -> Option<Id> {
    scalar(value.get(field)?).and_then(|s| Id::new(s).ok())
}
pub(crate) fn required_id(value: &Value, field: &str, outcome: &Outcome) -> Result<Id> {
    optional_id(value, field).ok_or_else(|| invalid(outcome))
}
pub(crate) fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) if n.is_i64() || n.is_u64() => Some(n.to_string()),
        _ => None,
    }
}

pub(crate) fn strict_json(text: &str) -> std::result::Result<Value, serde_json::Error> {
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

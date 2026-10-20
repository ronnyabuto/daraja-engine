//! Bounded HTTP boundary. Wire bodies and nested transport errors are never logged.
use crate::{auth::AuthError, crypto::Secret, identity::Id};
use reqwest::{Url, header::HeaderValue};
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub http_status: Option<u16>,
    pub request_id: Option<Id>,
    pub provider_code: Option<Id>,
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

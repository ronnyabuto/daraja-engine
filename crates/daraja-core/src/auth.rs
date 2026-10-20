//! Independent, cancellable OAuth actors. No financial request is retried here.
use crate::crypto::Secret;
use reqwest::{Client, Url, header::HeaderValue};
use serde::Deserialize;
use std::{
    fmt,
    marker::PhantomData,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{
    sync::{Mutex, Notify, RwLock},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

const REFRESH_TARGET: Duration = Duration::from_secs(50 * 60);
const FAILURE_BACKOFF: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const BODY_LIMIT: usize = 16 * 1024;

pub struct OAuthConfig {
    pub endpoint: Url,
    pub consumer_key: Secret,
    pub consumer_secret: Secret,
}
impl fmt::Debug for OAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OAuthConfig { endpoint: [REDACTED], consumer_key: [REDACTED], consumer_secret: [REDACTED] }")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthError {
    InvalidConfig,
    Cancelled,
    Transport,
    Rejected,
    InvalidResponse,
    RetryLimit,
    ExpiredToken,
}
impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidConfig => "invalid OAuth configuration",
            Self::Cancelled => "OAuth actor cancelled",
            Self::Transport => "OAuth transport failed",
            Self::Rejected => "OAuth request rejected",
            Self::InvalidResponse => "invalid OAuth token response",
            Self::RetryLimit => "OAuth 401 refresh limit reached",
            Self::ExpiredToken => "OAuth token expired",
        })
    }
}
impl std::error::Error for AuthError {}

#[derive(Clone, Debug)]
pub struct AccessToken(Arc<Token>);
#[derive(Debug)]
struct Token {
    value: Secret,
    refresh_at: Instant,
    expires_at: Instant,
}
impl AccessToken {
    pub fn authorization(&self) -> Result<HeaderValue, AuthError> {
        if Instant::now() >= self.0.expires_at {
            return Err(AuthError::ExpiredToken);
        }
        let mut header = HeaderValue::from_str(&format!("Bearer {}", self.0.value.expose()))
            .expect("validated token");
        header.set_sensitive(true);
        Ok(header)
    }
}
#[derive(Default)]
struct Cache {
    token: Option<AccessToken>,
    retry: Option<(Instant, AuthError)>,
}
struct Inner {
    config: OAuthConfig,
    client: Client,
    cache: RwLock<Cache>,
    refresh: Mutex<()>,
    changed: Notify,
    cancel: CancellationToken,
}
struct Lifetime(CancellationToken);
impl Drop for Lifetime {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub enum Collection {}
pub enum Disbursement {}
pub type CollectionApp = OAuthActor<Collection>;
pub type DisbursementApp = OAuthActor<Disbursement>;
pub struct OAuthActor<App> {
    inner: Arc<Inner>,
    lifetime: Arc<Lifetime>,
    app: PhantomData<fn() -> App>,
}
impl<App> Clone for OAuthActor<App> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            lifetime: self.lifetime.clone(),
            app: PhantomData,
        }
    }
}
impl<App> fmt::Debug for OAuthActor<App> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OAuthActor { credentials: [REDACTED] }")
    }
}
impl<App> OAuthActor<App> {
    pub(crate) fn matches_origin(&self, base: &Url) -> bool {
        self.inner.config.endpoint.origin() == base.origin()
    }

    /// Requires a Tokio runtime. Plain HTTP is accepted only for loopback fixtures.
    pub fn new(config: OAuthConfig) -> Result<Self, AuthError> {
        let local = config.endpoint.host_str().is_some_and(|host| {
            host.parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        });
        if (config.endpoint.scheme() != "https" && !(local && config.endpoint.scheme() == "http"))
            || !config.endpoint.username().is_empty()
            || config.endpoint.password().is_some()
            || config.endpoint.query().is_some()
            || config.endpoint.fragment().is_some()
            || config.consumer_key.expose().is_empty()
            || config.consumer_key.expose().contains(':')
            || config.consumer_secret.expose().is_empty()
        {
            return Err(AuthError::InvalidConfig);
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| AuthError::InvalidConfig)?;
        let cancel = CancellationToken::new();
        let inner = Arc::new(Inner {
            config,
            client,
            cache: RwLock::new(Cache::default()),
            refresh: Mutex::new(()),
            changed: Notify::new(),
            cancel: cancel.clone(),
        });
        tokio::spawn(background(Arc::downgrade(&inner), cancel.clone()));
        Ok(Self {
            inner,
            lifetime: Arc::new(Lifetime(cancel)),
            app: PhantomData,
        })
    }
    pub fn cancel(&self) {
        self.inner.cancel.cancel();
    }
    pub async fn token(&self) -> Result<AccessToken, AuthError> {
        tokio::select! { biased;
            _ = self.inner.cancel.cancelled() => Err(AuthError::Cancelled),
            result = self.inner.acquire(None) => result,
        }
    }
    /// One refresh per authorization attempt; transport owners decide whether replay is safe.
    pub async fn authorize(&self) -> Result<AuthorizationAttempt<App>, AuthError> {
        Ok(AuthorizationAttempt {
            actor: self.clone(),
            token: Some(self.token().await?),
            refreshed: false,
        })
    }
}

pub struct AuthorizationAttempt<App> {
    actor: OAuthActor<App>,
    token: Option<AccessToken>,
    refreshed: bool,
}
impl<App> fmt::Debug for AuthorizationAttempt<App> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthorizationAttempt")
            .field("token", &self.token)
            .field("refreshed", &self.refreshed)
            .finish()
    }
}
impl<App> AuthorizationAttempt<App> {
    pub fn token(&self) -> Option<&AccessToken> {
        self.token.as_ref()
    }
    pub async fn refresh_after_unauthorized(&mut self) -> Result<(), AuthError> {
        if self.refreshed {
            return Err(AuthError::RetryLimit);
        }
        self.refreshed = true;
        let rejected = self.token.take();
        self.token = Some(tokio::select! { biased;
            _ = self.actor.inner.cancel.cancelled() => return Err(AuthError::Cancelled),
            result = self.actor.inner.acquire(rejected.as_ref()) => result?,
        });
        Ok(())
    }
}
impl Inner {
    async fn cached(
        &self,
        rejected: Option<&AccessToken>,
    ) -> Result<Option<AccessToken>, AuthError> {
        let cache = self.cache.read().await;
        let now = Instant::now();
        if let Some((retry_at, error)) = cache.retry
            && now < retry_at
        {
            return Err(error);
        }
        Ok(cache
            .token
            .as_ref()
            .filter(|token| {
                now < token.0.refresh_at
                    && now < token.0.expires_at
                    && !rejected.is_some_and(|old| Arc::ptr_eq(&old.0, &token.0))
            })
            .cloned())
    }
    async fn acquire(&self, rejected: Option<&AccessToken>) -> Result<AccessToken, AuthError> {
        if let Some(token) = self.cached(rejected).await? {
            return Ok(token);
        }
        let _refresh = self.refresh.lock().await;
        if let Some(token) = self.cached(rejected).await? {
            return Ok(token);
        }
        // Invalidate before I/O: a cancelled or failed refresh cannot revive a rejected token.
        self.cache.write().await.token = None;
        let result = tokio::select! { biased;
            _ = self.cancel.cancelled() => Err(AuthError::Cancelled),
            result = tokio::time::timeout(REQUEST_TIMEOUT, self.fetch()) => result.unwrap_or(Err(AuthError::Transport)),
        };
        let mut cache = self.cache.write().await;
        match &result {
            Ok(token) => {
                cache.token = Some(token.clone());
                cache.retry = None;
            }
            Err(error) => {
                cache.retry = Some((Instant::now() + FAILURE_BACKOFF, *error));
            }
        }
        self.changed.notify_one();
        result
    }
    async fn fetch(&self) -> Result<AccessToken, AuthError> {
        // Anchor expiry before network latency, not at response receipt.
        let started = Instant::now();
        let mut response = self
            .client
            .get(self.config.endpoint.clone())
            .query(&[("grant_type", "client_credentials")])
            .basic_auth(
                self.config.consumer_key.expose(),
                Some(self.config.consumer_secret.expose()),
            )
            .send()
            .await
            .map_err(|_| AuthError::Transport)?;
        if !response.status().is_success() {
            return Err(AuthError::Rejected);
        }
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| AuthError::Transport)? {
            if bytes.len() + chunk.len() > BODY_LIMIT {
                return Err(AuthError::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        let wire: TokenResponse =
            serde_json::from_slice(&bytes).map_err(|_| AuthError::InvalidResponse)?;
        let lifetime = match wire.expires_in {
            Expiry::Number(n) => n,
            Expiry::Text(s) => s.parse().map_err(|_| AuthError::InvalidResponse)?,
        };
        let lifetime = Duration::from_secs(lifetime);
        let expires_at = started
            .checked_add(lifetime)
            .ok_or(AuthError::InvalidResponse)?;
        let refresh_at = started
            .checked_add(REFRESH_TARGET.min(lifetime - lifetime / 6))
            .ok_or(AuthError::InvalidResponse)?;
        let value = wire.access_token;
        if value.expose().is_empty()
            || !value
                .expose()
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b))
            || Instant::now() >= refresh_at
            || Instant::now() >= expires_at
        {
            return Err(AuthError::InvalidResponse);
        }
        Ok(AccessToken(Arc::new(Token {
            value,
            refresh_at,
            expires_at,
        })))
    }
}
#[derive(Deserialize)]
struct TokenResponse {
    #[serde(deserialize_with = "deserialize_secret")]
    access_token: Secret,
    expires_in: Expiry,
}
fn deserialize_secret<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Secret, D::Error> {
    String::deserialize(d).map(Secret::new)
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Expiry {
    Number(u64),
    Text(String),
}

async fn background(weak: Weak<Inner>, cancel: CancellationToken) {
    loop {
        let Some(inner) = weak.upgrade() else {
            return;
        };
        let deadline = {
            let cache = inner.cache.read().await;
            cache
                .retry
                .map(|(at, _)| at)
                .or_else(|| cache.token.as_ref().map(|token| token.0.refresh_at))
        };
        tokio::select! { biased;
            _ = cancel.cancelled() => return,
            _ = inner.changed.notified() => continue,
            _ = async { match deadline { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => {}
        }
        let _ = inner.acquire(None).await;
    }
}

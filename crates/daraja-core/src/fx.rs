//! Pluggable quotes; missing, stale or malformed data never enables sat.
use crate::{Error, Timestamp, amount::FxSnapshot, identity::Id};
use std::{future::Future, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxError {
    Unavailable,
    Malformed,
    Stale,
    Configuration,
    Arithmetic(Error),
}
impl From<Error> for FxError {
    fn from(e: Error) -> Self {
        Self::Arithmetic(e)
    }
}

pub trait FxOracle: Send + Sync {
    fn quote(&self, now: Timestamp) -> impl Future<Output = Result<FxSnapshot, FxError>> + Send;
}

#[derive(Clone)]
pub struct OfflineOracle {
    pub snapshot: Option<FxSnapshot>,
}
impl FxOracle for OfflineOracle {
    async fn quote(&self, now: Timestamp) -> Result<FxSnapshot, FxError> {
        let s = self.snapshot.clone().ok_or(FxError::Unavailable)?;
        s.validate_at(now)?;
        Ok(s)
    }
}

/// Exact positive fixed decimal; signs, exponents and more than 8 places are rejected.
pub fn decimal_ratio(s: &str) -> Result<(u64, u64), FxError> {
    let mut parts = s.split('.');
    let whole = parts.next().ok_or(FxError::Malformed)?;
    let fraction = parts.next();
    if parts.next().is_some() || whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return Err(FxError::Malformed);
    }
    let frac = fraction.unwrap_or("");
    if (fraction.is_some() && frac.is_empty())
        || frac.len() > 8
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(FxError::Malformed);
    }
    let scale = 10u64.pow(frac.len() as u32);
    let value = whole
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(scale))
        .and_then(|n| {
            n.checked_add(if frac.is_empty() {
                0
            } else {
                frac.parse::<u64>().ok()?
            })
        })
        .ok_or(FxError::Malformed)?;
    if value == 0 {
        return Err(FxError::Malformed);
    }
    Ok((value, scale))
}

/// Coinbase v2 exchange-rates. Freshness is observation age, not a market timestamp.
/// Production uses HTTPS; HTTP is accepted only on loopback for boundary fixtures.
pub struct CoinbaseOracle {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    ttl: u64,
    max_age: u64,
}
impl CoinbaseOracle {
    pub fn new(endpoint: &str, ttl: u64, max_age: u64, timeout: Duration) -> Result<Self, FxError> {
        let endpoint = reqwest::Url::parse(endpoint).map_err(|_| FxError::Configuration)?;
        let loopback = matches!(
            endpoint.host_str(),
            Some("127.0.0.1" | "[::1]" | "localhost")
        );
        if (endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback))
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || ttl == 0
            || ttl > max_age
            || max_age > 300
            || timeout.is_zero()
            || timeout > Duration::from_secs(20)
        {
            return Err(FxError::Configuration);
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| FxError::Configuration)?;
        Ok(Self {
            client,
            endpoint,
            ttl,
            max_age,
        })
    }
    pub fn parse(
        body: &[u8],
        now: Timestamp,
        expires_at: Timestamp,
    ) -> Result<FxSnapshot, FxError> {
        #[derive(serde::Deserialize)]
        struct Response {
            data: Data,
        }
        #[derive(serde::Deserialize)]
        struct Data {
            currency: String,
            rates: Rates,
        }
        #[derive(serde::Deserialize)]
        struct Rates {
            #[serde(rename = "KES")]
            kes: Option<String>,
        }
        let r: Response = serde_json::from_slice(body).map_err(|_| FxError::Malformed)?;
        if r.data.currency != "BTC" {
            return Err(FxError::Malformed);
        }
        let (price, scale) =
            decimal_ratio(r.data.rates.kes.as_deref().ok_or(FxError::Unavailable)?)?;
        // 1 BTC = price/scale KES; 1 KES minor = 1_000_000*scale/price sat.
        let s = FxSnapshot {
            source: Id::new("coinbase-v2-BTC-KES")?,
            numerator: scale.checked_mul(1_000_000).ok_or(Error::Overflow)?,
            denominator: price,
            expires_at,
        };
        s.validate_at(now)?;
        Ok(s)
    }
}
impl FxOracle for CoinbaseOracle {
    async fn quote(&self, now: Timestamp) -> Result<FxSnapshot, FxError> {
        let started = tokio::time::Instant::now();
        let mut r = self
            .client
            .get(self.endpoint.clone())
            .query(&[("currency", "BTC")])
            .send()
            .await
            .map_err(|_| FxError::Unavailable)?;
        if !r.status().is_success() {
            return Err(FxError::Unavailable);
        }
        let age = match r.headers().get("age") {
            Some(v) => v
                .to_str()
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or(FxError::Malformed)?,
            None => 0,
        };
        if age >= self.max_age {
            return Err(FxError::Stale);
        }
        let mut body = Vec::new();
        while let Some(chunk) = r.chunk().await.map_err(|_| FxError::Unavailable)? {
            if body.len().saturating_add(chunk.len()) > 64 * 1024 {
                return Err(FxError::Malformed);
            }
            body.extend_from_slice(&chunk);
        }
        let lifetime = self.ttl.min(self.max_age - age);
        if started.elapsed().as_secs() >= lifetime {
            return Err(FxError::Stale);
        }
        let expiry = Timestamp(now.0.checked_add(lifetime).ok_or(Error::Overflow)?);
        Self::parse(&body, now, expiry)
    }
}

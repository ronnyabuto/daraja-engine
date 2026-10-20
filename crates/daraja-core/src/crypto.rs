//! Daraja wire cryptography. Certificate provenance is an operator trust boundary.
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, FixedOffset, Utc};
use rsa::{Pkcs1v15Encrypt, RsaPublicKey, pkcs8::DecodePublicKey, rand_core::OsRng};
use std::{fmt, path::PathBuf};
use x509_cert::{
    Certificate,
    der::{Decode, DecodePem, Encode},
};
use zeroize::Zeroizing;

/// Explicit exposure is required at the wire boundary; Debug never prints the value.
#[derive(Clone)]
pub struct Secret(Zeroizing<String>);
impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptoError {
    InvalidCertificate,
    InvalidRsaKey,
    CertificateRead,
    Encryption,
    EnvironmentMismatch,
}
impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidCertificate => "invalid X.509 certificate (expected DER or PEM)",
            Self::InvalidRsaKey => "certificate does not contain a supported RSA public key",
            Self::CertificateRead => "could not read configured certificate",
            Self::Encryption => "SecurityCredential encryption failed",
            Self::EnvironmentMismatch => {
                "certificate environment does not match request environment"
            }
        })
    }
}
impl std::error::Error for CryptoError {}

pub trait Clock {
    fn now(&self) -> DateTime<Utc>;
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Debug)]
pub struct StkPassword {
    pub timestamp: String,
    pub password: Secret,
}
pub fn stk_password(shortcode: &str, passkey: &Secret, clock: &impl Clock) -> StkPassword {
    let eat = FixedOffset::east_opt(3 * 3600).expect("fixed EAT offset");
    let timestamp = clock
        .now()
        .with_timezone(&eat)
        .format("%Y%m%d%H%M%S")
        .to_string();
    let plaintext = Zeroizing::new(format!("{shortcode}{}{timestamp}", passkey.expose()));
    StkPassword {
        password: Secret::new(STANDARD.encode(plaintext.as_bytes())),
        timestamp,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Environment {
    Sandbox,
    Production,
}
#[derive(Debug)]
pub struct CertificateConfig {
    pub sandbox: PathBuf,
    pub production: PathBuf,
}
impl CertificateConfig {
    /// Paths must point to certificates obtained and authenticated by the operator.
    pub fn load(&self, environment: Environment) -> Result<SecurityCredentialKey, CryptoError> {
        let path = match environment {
            Environment::Sandbox => &self.sandbox,
            Environment::Production => &self.production,
        };
        let bytes = std::fs::read(path).map_err(|_| CryptoError::CertificateRead)?;
        SecurityCredentialKey::from_certificate(environment, &bytes)
    }
}
#[derive(Debug)]
pub struct SecurityCredentialKey {
    environment: Environment,
    key: RsaPublicKey,
}
impl SecurityCredentialKey {
    /// Parses a configured trust anchor, not a WebPKI chain or an authenticity attestation.
    pub fn from_certificate(environment: Environment, bytes: &[u8]) -> Result<Self, CryptoError> {
        let cert = if bytes.starts_with(b"-----BEGIN") {
            Certificate::from_pem(bytes)
        } else {
            Certificate::from_der(bytes)
        }
        .map_err(|_| CryptoError::InvalidCertificate)?;
        let spki = cert
            .tbs_certificate
            .subject_public_key_info
            .to_der()
            .map_err(|_| CryptoError::InvalidCertificate)?;
        let key =
            RsaPublicKey::from_public_key_der(&spki).map_err(|_| CryptoError::InvalidRsaKey)?;
        Ok(Self { environment, key })
    }
    pub fn encrypt(
        &self,
        environment: Environment,
        password: &Secret,
    ) -> Result<Secret, CryptoError> {
        if self.environment != environment {
            return Err(CryptoError::EnvironmentMismatch);
        }
        let encrypted = self
            .key
            .encrypt(&mut OsRng, Pkcs1v15Encrypt, password.expose().as_bytes())
            .map_err(|_| CryptoError::Encryption)?;
        Ok(Secret::new(STANDARD.encode(encrypted)))
    }
}

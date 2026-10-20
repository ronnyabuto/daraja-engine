use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use daraja_core::crypto::{
    CertificateConfig, Clock, CryptoError, Environment, Secret, SecurityCredentialKey, stk_password,
};
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey, pkcs8::DecodePrivateKey};

struct FixedClock(&'static str);
impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0.parse().unwrap()
    }
}
const CERT: &[u8] = include_bytes!("fixtures/rsa-cert.txt");
const DER: &[u8] = include_bytes!("fixtures/rsa-cert.der");

#[test]
fn stk_eat_known_vectors_and_date_rollover() {
    let passkey = Secret::new("synthetic-passkey");
    for (utc, eat, expected) in [
        (
            "2026-10-20T09:34:56Z",
            "20261020123456",
            "MTc0Mzc5c3ludGhldGljLXBhc3NrZXkyMDI2MTAyMDEyMzQ1Ng==",
        ),
        (
            "2026-12-31T22:00:00Z",
            "20270101010000",
            "MTc0Mzc5c3ludGhldGljLXBhc3NrZXkyMDI3MDEwMTAxMDAwMA==",
        ),
    ] {
        let generated = stk_password("174379", &passkey, &FixedClock(utc));
        assert_eq!(generated.timestamp, eat);
        assert_eq!(generated.password.expose(), expected);
        assert!(!format!("{generated:?}").contains(expected));
        assert!(!format!("{generated:?}").contains(passkey.expose()));
    }
}

#[test]
fn randomized_pkcs1_credentials_decrypt_to_original_plaintext_for_pem_and_der() {
    let private =
        RsaPrivateKey::from_pkcs8_pem(include_str!("fixtures/rsa-private-key.txt")).unwrap();
    let password = Secret::new("synthetic-initiator-password");
    for cert in [CERT, DER] {
        let key = SecurityCredentialKey::from_certificate(Environment::Sandbox, cert).unwrap();
        let first = key.encrypt(Environment::Sandbox, &password).unwrap();
        let second = key.encrypt(Environment::Sandbox, &password).unwrap();
        assert_ne!(first.expose(), second.expose());
        for encoded in [first, second] {
            let ciphertext = STANDARD.decode(encoded.expose()).unwrap();
            assert_eq!(ciphertext.len(), 256);
            assert_eq!(
                private.decrypt(Pkcs1v15Encrypt, &ciphertext).unwrap(),
                password.expose().as_bytes()
            );
            assert!(!format!("{encoded:?}").contains(encoded.expose()));
        }
        assert_eq!(
            key.encrypt(Environment::Production, &password).unwrap_err(),
            CryptoError::EnvironmentMismatch
        );
        assert_eq!(
            key.encrypt(Environment::Sandbox, &Secret::new("x".repeat(246)))
                .unwrap_err(),
            CryptoError::Encryption
        );
    }
}

#[test]
fn environment_specific_files_invalid_certificates_and_non_rsa_keys() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = dir.path().join("sandbox.cer");
    let production = dir.path().join("production.cer");
    std::fs::write(&sandbox, CERT).unwrap();
    std::fs::write(&production, DER).unwrap();
    let config = CertificateConfig {
        sandbox,
        production,
    };
    for environment in [Environment::Sandbox, Environment::Production] {
        assert!(
            config
                .load(environment)
                .unwrap()
                .encrypt(environment, &Secret::new("synthetic"))
                .is_ok()
        );
    }
    std::fs::write(&config.production, b"invalid").unwrap();
    assert!(config.load(Environment::Sandbox).is_ok());
    assert_eq!(
        config.load(Environment::Production).unwrap_err(),
        CryptoError::InvalidCertificate
    );
    let missing = CertificateConfig {
        sandbox: dir.path().join("missing"),
        production: dir.path().join("missing"),
    };
    assert_eq!(
        missing.load(Environment::Sandbox).unwrap_err(),
        CryptoError::CertificateRead
    );
    for bytes in [
        b"invalid-cert-secret".as_slice(),
        &DER[..DER.len() / 2],
        include_bytes!("fixtures/rsa-private-key.txt").as_slice(),
    ] {
        let error =
            SecurityCredentialKey::from_certificate(Environment::Sandbox, bytes).unwrap_err();
        assert_eq!(error, CryptoError::InvalidCertificate);
        assert!(error.to_string().contains("X.509"));
        assert!(!error.to_string().contains("invalid-cert-secret"));
    }
    assert_eq!(
        SecurityCredentialKey::from_certificate(
            Environment::Sandbox,
            include_bytes!("fixtures/ec-cert.txt")
        )
        .unwrap_err(),
        CryptoError::InvalidRsaKey
    );
}

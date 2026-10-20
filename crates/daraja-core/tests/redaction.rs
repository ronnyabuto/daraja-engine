use axum::{Router, http::StatusCode, routing::get};
use base64::{Engine, engine::general_purpose::STANDARD};
use daraja_core::{
    auth::{AuthError, CollectionApp, OAuthConfig},
    crypto::{Clock, Environment, Secret, SecurityCredentialKey, stk_password},
};
use std::{
    io::Write,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        "2026-10-20T09:34:56Z".parse().unwrap()
    }
}

#[tokio::test]
async fn debug_errors_and_trace_output_omit_secrets_at_http_and_crypto_boundaries() {
    let capture = Capture::default();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .without_time()
            .with_writer(capture.clone())
            .finish(),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/ok", get(|| async { axum::Json(serde_json::json!({"access_token":"synthetic-bearer-redaction-marker", "expires_in":3600})) }))
        .route("/bad", get(|| async { (StatusCode::UNAUTHORIZED, "synthetic-provider-error-secret") }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let config = |path: &str| OAuthConfig {
        endpoint: format!("{url}/{path}").parse().unwrap(),
        consumer_key: Secret::new("synthetic-consumer-key-marker"),
        consumer_secret: Secret::new("synthetic-consumer-secret-marker"),
    };
    tracing::debug!(config = ?config("ok"));
    let actor = CollectionApp::new(config("ok")).unwrap();
    let token = actor.token().await.unwrap();
    assert!(token.authorization().unwrap().is_sensitive());
    tracing::debug!(?actor, ?token, header = ?token.authorization().unwrap(), attempt = ?actor.authorize().await.unwrap());
    let bad = CollectionApp::new(config("bad")).unwrap();
    let error = bad.token().await.unwrap_err();
    assert_eq!(error, AuthError::Rejected);
    tracing::error!(?error, message = %error);
    let passkey = Secret::new("synthetic-passkey-marker");
    let stk = stk_password("174379", &passkey, &FixedClock);
    let password = Secret::new("synthetic-initiator-password-marker");
    let key = SecurityCredentialKey::from_certificate(
        Environment::Sandbox,
        include_bytes!("fixtures/rsa-cert.txt"),
    )
    .unwrap();
    let encrypted = key.encrypt(Environment::Sandbox, &password).unwrap();
    let crypto_error = key
        .encrypt(
            Environment::Sandbox,
            &Secret::new("synthetic-too-long-password-marker".repeat(30)),
        )
        .unwrap_err();
    tracing::debug!(?passkey, ?password, ?stk, ?key, ?encrypted, ?crypto_error, message = %crypto_error);
    actor.cancel();
    bad.cancel();
    server.abort();
    tokio::task::yield_now().await;
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("REDACTED"));
    for secret in [
        "synthetic-consumer-key-marker",
        "synthetic-consumer-secret-marker",
        "synthetic-bearer-redaction-marker",
        "synthetic-provider-error-secret",
        "synthetic-passkey-marker",
        "synthetic-initiator-password-marker",
        "synthetic-too-long-password-marker",
        stk.password.expose(),
        encrypted.expose(),
        &STANDARD.encode("synthetic-consumer-key-marker:synthetic-consumer-secret-marker"),
    ] {
        assert!(
            !output.contains(secret),
            "secret marker leaked into captured tracing"
        );
    }
}

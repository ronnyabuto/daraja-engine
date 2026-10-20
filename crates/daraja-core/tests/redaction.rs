use axum::{
    Router,
    http::StatusCode,
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use daraja_core::{
    amount::{Amount, Unit},
    auth::{AuthError, CollectionApp, DisbursementApp, OAuthConfig},
    collection::{
        C2bRegistration, C2bResponsePolicy, CollectionClient, StatusAcknowledgment,
        StatusCredentials, StatusIdentifierType, StatusLookup, StatusPersistence, StatusPrepared,
        StatusRequest, StkConfig, StkSubmission, StkTransactionType, TransactionStatusClient,
    },
    crypto::{Clock, Environment, Secret, SecurityCredentialKey, stk_password},
    identity::Id,
    payout::{
        BalanceAccount, BalanceClient, BalancePersistence, BalancePrepared, BalanceRequest,
        Capabilities, Destination, PayoutAcknowledgment, PayoutClient, PayoutPersistence,
        PayoutRequest, PreparedPayout, Remitter,
    },
    transport::{ReqwestTransport, Result as TransportResult},
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
    let provider_error = || async {
        axum::Json(
            serde_json::json!({"errorCode":"400.001", "errorMessage":"synthetic-provider-error-secret"}),
        )
    };
    let app = Router::new()
        .route("/mpesa/stkpush/v1/processrequest", post(provider_error))
        .route("/mpesa/c2b/v1/registerurl", post(provider_error))
        .route("/mpesa/transactionstatus/v1/query", post(provider_error))
        .route("/mpesa/b2b/v1/paymentrequest", post(provider_error))
        .route("/mpesa/b2c/v3/paymentrequest", post(provider_error))
        .route("/mpesa/accountbalance/v1/query", post(provider_error))
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
    let transport = || {
        ReqwestTransport::new(
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(2),
        )
        .unwrap()
    };
    let collection = CollectionClient::new(
        transport(),
        url.parse().unwrap(),
        actor.clone(),
        FixedClock,
        StkConfig {
            shortcode: Id::new("174379").unwrap(),
            passkey: passkey.clone(),
        },
    )
    .unwrap();
    let request = StkSubmission {
        amount: Amount::new(100, Unit::KesMinor),
        phone: Secret::new("254799888777"),
        callback_url: Secret::new(
            "https://example.invalid/callback-secret-marker?secret=query-secret-marker",
        ),
        account_reference: Id::new("ABCD1234").unwrap(),
        description: Secret::new("description-secret-marker"),
        transaction_type: StkTransactionType::CustomerPayBillOnline,
    };
    let error = collection.submit_stk(&request).await.unwrap_err();
    tracing::debug!(?request, ?error);
    let registration = C2bRegistration {
        shortcode: Id::new("600000").unwrap(),
        confirmation_url: Secret::new("https://example.invalid/confirmation-secret-marker"),
        validation_url: Secret::new("https://example.invalid/validation-secret-marker"),
        response_policy: C2bResponsePolicy::Cancelled,
    };
    let error = collection.register_c2b(&registration).await.unwrap_err();
    tracing::debug!(?registration, ?error);
    struct Hooks;
    impl StatusPersistence for Hooks {
        async fn prepare(&self, _: StatusPrepared<'_>) -> TransportResult<()> {
            Ok(())
        }
        async fn acknowledge(&self, _: &Id, _: &StatusAcknowledgment) -> TransportResult<()> {
            Ok(())
        }
    }
    let status = TransactionStatusClient::new(
        transport(),
        url.parse().unwrap(),
        StatusCredentials::Collection(actor.clone()),
        FixedClock,
    )
    .unwrap();
    let request = StatusRequest {
        local_query_id: Id::new("local-query").unwrap(),
        lookup: StatusLookup::Receipt(Id::new("ABC1234567").unwrap()),
        initiator: password.clone(),
        security_credential: encrypted.clone(),
        party_a: Id::new("600000").unwrap(),
        identifier_type: StatusIdentifierType::Shortcode,
        result_url: Secret::new("https://example.invalid/result-secret-marker"),
        timeout_url: Secret::new("https://example.invalid/timeout-secret-marker"),
        remarks: Secret::new("remarks-secret-marker"),
        occasion: Secret::new("occasion-secret-marker"),
    };
    let error = status.request(&request, &Hooks).await.unwrap_err();
    tracing::debug!(?request, ?error);
    struct PayoutHooks;
    impl PayoutPersistence for PayoutHooks {
        async fn prepare(&self, payout: &PreparedPayout) -> TransportResult<()> {
            tracing::debug!(?payout);
            Ok(())
        }
        async fn acknowledge(&self, _: &PayoutAcknowledgment) -> TransportResult<()> {
            Ok(())
        }
    }
    let remitter = || {
        Remitter::new(
            Environment::Sandbox,
            &key,
            Secret::new("synthetic-remitter-marker"),
            &password,
            Id::new("600000").unwrap(),
        )
        .unwrap()
    };
    let payout_actor = DisbursementApp::new(config("ok")).unwrap();
    let mut payout = PayoutClient::new(
        transport(),
        url.parse().unwrap(),
        payout_actor.clone(),
        remitter(),
        250000,
    )
    .unwrap();
    payout
        .configure_capabilities(
            Capabilities {
                buy_goods: true,
                pay_bill: true,
                b2b_caller_id: true,
                ..Capabilities::default()
            },
            None,
        )
        .unwrap();
    for destination in ["TILL:123456", "PAYBILL:654321:PRIVATE123", "254799888777"] {
        let request = PayoutRequest {
            caller_originator_conversation_id: Id::new("synthetic-caller-marker").unwrap(),
            destination: Destination::parse(destination).unwrap(),
            amount: Amount::new(100, Unit::KesMinor),
            result_url: Secret::new("https://example.invalid/result-secret-marker"),
            timeout_url: Secret::new("https://example.invalid/timeout-secret-marker"),
        };
        let error = payout.submit(&request, &PayoutHooks).await.unwrap_err();
        tracing::debug!(?request, ?error, message = %error);
    }
    struct BalanceHooks;
    impl BalancePersistence for BalanceHooks {
        async fn prepare(&self, record: BalancePrepared<'_>) -> TransportResult<()> {
            tracing::debug!(?record);
            Ok(())
        }
        async fn acknowledge(&self, _: &Id, _: &StatusAcknowledgment) -> TransportResult<()> {
            Ok(())
        }
    }
    let balance = BalanceClient::new(
        transport(),
        url.parse().unwrap(),
        StatusCredentials::Collection(actor.clone()),
        remitter(),
        FixedClock,
    )
    .unwrap();
    let request = BalanceRequest {
        local_query_id: Id::new("synthetic-balance-marker").unwrap(),
        account: BalanceAccount::Shortcode(Id::new("600000").unwrap()),
        result_url: Secret::new("https://example.invalid/result-secret-marker"),
        timeout_url: Secret::new("https://example.invalid/timeout-secret-marker"),
    };
    let error = balance.request(&request, &BalanceHooks).await.unwrap_err();
    tracing::debug!(?request, ?error);
    payout_actor.cancel();
    actor.cancel();
    bad.cancel();
    server.abort();
    tokio::task::yield_now().await;
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("REDACTED"));
    for secret in [
        "synthetic-remitter-marker",
        "synthetic-caller-marker",
        "synthetic-balance-marker",
        "PRIVATE123",
        "callback-secret-marker",
        "query-secret-marker",
        "description-secret-marker",
        "254799888777",
        "confirmation-secret-marker",
        "validation-secret-marker",
        "result-secret-marker",
        "timeout-secret-marker",
        "remarks-secret-marker",
        "occasion-secret-marker",
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

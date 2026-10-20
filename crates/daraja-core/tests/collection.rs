use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use daraja_core::{
    amount::{Amount, Unit},
    auth::{CollectionApp, DisbursementApp, OAuthConfig},
    collection::*,
    crypto::{Clock, Secret},
    identity::Id,
    transport::{
        ClientError, ErrorKind, HttpRequest, HttpTransport, Phase, ReqwestTransport, Result,
    },
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

fn id(s: &str) -> Id {
    Id::new(s).unwrap()
}
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        "2026-10-20T09:34:56Z".parse().unwrap()
    }
}
#[derive(Clone)]
enum Reply {
    Json(u16, Value),
    Raw(u16, &'static str),
    HeadersHang,
    BodyHang,
    Redirect,
}
struct Seen {
    path: String,
    headers: HeaderMap,
    body: Value,
}
struct Provider {
    replies: Mutex<VecDeque<Reply>>,
    auth_calls: AtomicUsize,
    seen: Mutex<Vec<Seen>>,
    prepared: AtomicBool,
    require_prepared: AtomicBool,
    release: Notify,
}
struct Server {
    state: Arc<Provider>,
    base: reqwest::Url,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn auth(State(state): State<Arc<Provider>>, request: Request) -> axum::Json<Value> {
    state.auth_calls.fetch_add(1, Ordering::SeqCst);
    let name = request.uri().path().trim_start_matches('/').to_owned();
    let credentials = request.headers()["authorization"].to_str().unwrap();
    use base64::Engine;
    assert_eq!(
        credentials,
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{name}:secret"))
        )
    );
    axum::Json(json!({"access_token":format!("{name}-token"), "expires_in":"3600"}))
}
async fn endpoint(State(state): State<Arc<Provider>>, request: Request) -> Response {
    if state.require_prepared.load(Ordering::SeqCst) {
        assert!(state.prepared.load(Ordering::SeqCst));
    }
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 65536).await.unwrap();
    state.seen.lock().unwrap().push(Seen {
        path: parts.uri.path().to_owned(),
        headers: parts.headers,
        body: serde_json::from_slice(&bytes).unwrap(),
    });
    let reply = state
        .replies
        .lock()
        .unwrap()
        .pop_front()
        .expect("unexpected replay");
    let mut response = match reply {
        Reply::Json(status, body) => {
            (StatusCode::from_u16(status).unwrap(), axum::Json(body)).into_response()
        }
        Reply::Raw(status, body) => (StatusCode::from_u16(status).unwrap(), body).into_response(),
        Reply::HeadersHang => {
            state.release.notified().await;
            StatusCode::OK.into_response()
        }
        Reply::BodyHang => {
            let stream = std::future::pending::<()>();
            // A real HTTP response with headers but an unfinished body.
            let (mut sender, body) = tokio::io::duplex(64);
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                sender.write_all(b"{").await.unwrap();
                stream.await;
            });
            Body::from_stream(tokio_util::io::ReaderStream::new(body)).into_response()
        }
        Reply::Redirect => Response::builder()
            .status(307)
            .header("location", "/unexpected")
            .body(Body::empty())
            .unwrap(),
    };
    response
        .headers_mut()
        .insert("x-request-id", "header-request-id".parse().unwrap());
    response
}
impl Server {
    async fn new(replies: Vec<Reply>) -> Self {
        let state = Arc::new(Provider {
            replies: Mutex::new(replies.into()),
            auth_calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
            prepared: AtomicBool::new(false),
            require_prepared: AtomicBool::new(false),
            release: Notify::new(),
        });
        let app = Router::new()
            .route("/collection", get(auth))
            .route("/disbursement", get(auth))
            .route("/mpesa/stkpush/v1/processrequest", post(endpoint))
            .route("/mpesa/stkpushquery/v1/query", post(endpoint))
            .route("/mpesa/c2b/v1/registerurl", post(endpoint))
            .route("/mpesa/transactionstatus/v1/query", post(endpoint))
            .route("/mpesa/b2c/v3/paymentrequest", post(endpoint))
            .route("/mpesa/b2b/v1/paymentrequest", post(endpoint))
            .route("/mpesa/accountbalance/v1/query", post(endpoint))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { state, base, task }
    }
    fn config(&self, name: &str) -> OAuthConfig {
        OAuthConfig {
            endpoint: self.base.join(name).unwrap(),
            consumer_key: Secret::new(name),
            consumer_secret: Secret::new("secret"),
        }
    }
    fn collection(&self) -> CollectionClient<ReqwestTransport, FixedClock> {
        self.collection_timeout(Duration::from_secs(2))
    }
    fn collection_timeout(
        &self,
        deadline: Duration,
    ) -> CollectionClient<ReqwestTransport, FixedClock> {
        CollectionClient::new(
            ReqwestTransport::new(deadline / 2, deadline).unwrap(),
            self.base.clone(),
            CollectionApp::new(self.config("collection")).unwrap(),
            FixedClock,
            StkConfig {
                shortcode: id("174379"),
                passkey: Secret::new("test-passkey"),
            },
        )
        .unwrap()
    }
    fn status(&self, collection: bool) -> TransactionStatusClient<ReqwestTransport, FixedClock> {
        let credentials = if collection {
            StatusCredentials::Collection(CollectionApp::new(self.config("collection")).unwrap())
        } else {
            StatusCredentials::Disbursement(
                DisbursementApp::new(self.config("disbursement")).unwrap(),
            )
        };
        TransactionStatusClient::new(
            ReqwestTransport::new(Duration::from_millis(50), Duration::from_millis(300)).unwrap(),
            self.base.clone(),
            credentials,
            FixedClock,
        )
        .unwrap()
    }
    fn count(&self) -> usize {
        self.state.seen.lock().unwrap().len()
    }
}
fn stk_ack() -> Value {
    json!({"ResponseCode":"0", "ResponseDescription":"Success. Request accepted for processing", "MerchantRequestID":"merchant-1", "CheckoutRequestID":"checkout-1", "CustomerMessage":"Success"})
}
fn stk_query() -> Value {
    json!({"ResponseCode":0,"ResponseDescription":"The service request has been accepted successfully", "MerchantRequestID":"merchant-1", "CheckoutRequestID":"checkout-1", "ResultCode":"1032", "ResultDesc":"Request cancelled by user"})
}
fn status_ack() -> Value {
    json!({"OriginatorConversationID":"originator-1", "ConversationID":"conversation-1", "ResponseCode":"0", "ResponseDescription":"Accept the service request successfully."})
}
fn submission() -> StkSubmission {
    StkSubmission {
        amount: Amount::new(12300, Unit::KesMinor),
        phone: Secret::new("254712345678"),
        callback_url: Secret::new(
            "https://example.invalid/stk/callback-url-secret?token=query-secret",
        ),
        account_reference: id("ABCD1234"),
        description: Secret::new("deposit"),
        transaction_type: StkTransactionType::CustomerPayBillOnline,
    }
}
fn registration() -> C2bRegistration {
    C2bRegistration {
        shortcode: id("600000"),
        confirmation_url: Secret::new("https://example.invalid/confirmation/confirmation-secret"),
        validation_url: Secret::new("https://example.invalid/validation/validation-secret"),
        response_policy: C2bResponsePolicy::default(),
    }
}
fn status_request() -> StatusRequest {
    StatusRequest {
        local_query_id: id("query-1"),
        lookup: StatusLookup::Receipt(id("QAB1234567")),
        initiator: Secret::new("initiator-secret"),
        security_credential: Secret::new("encrypted-credential-secret"),
        party_a: id("600000"),
        identifier_type: StatusIdentifierType::Shortcode,
        result_url: Secret::new("https://example.invalid/status/result-secret"),
        timeout_url: Secret::new("https://example.invalid/status/timeout-secret"),
        remarks: Secret::new("reconcile"),
        occasion: Secret::new("query"),
    }
}
struct Hooks {
    provider: Arc<Provider>,
    rows: Mutex<Vec<(String, String)>>,
    fail_prepare: bool,
    fail_ack: bool,
}
impl Hooks {
    fn new(server: &Server) -> Self {
        Self {
            provider: server.state.clone(),
            rows: Mutex::new(Vec::new()),
            fail_prepare: false,
            fail_ack: false,
        }
    }
}
impl StatusPersistence for Hooks {
    async fn prepare(&self, record: StatusPrepared<'_>) -> Result<()> {
        if self.fail_prepare {
            return Err(ClientError::new(ErrorKind::Persistence));
        }
        assert_eq!(record.created_at, FixedClock.now());
        assert_eq!(record.local_query_id.as_str(), "query-1");
        self.rows
            .lock()
            .unwrap()
            .push(("prepared".into(), record.local_query_id.as_str().into()));
        self.provider.prepared.store(true, Ordering::SeqCst);
        Ok(())
    }
    async fn acknowledge(&self, local: &Id, ack: &StatusAcknowledgment) -> Result<()> {
        if self.fail_ack {
            return Err(ClientError::new(ErrorKind::Persistence));
        }
        assert_eq!(local.as_str(), "query-1");
        self.rows.lock().unwrap().push((
            ack.originator_conversation_id.as_str().into(),
            ack.conversation_id.as_str().into(),
        ));
        Ok(())
    }
}
#[tokio::test]
async fn four_wire_contracts_use_exact_fields_paths_and_credential_actors() {
    use base64::Engine;
    let server = Server::new(vec![
        Reply::Json(200, stk_ack()),
        Reply::Json(200, stk_query()),
        Reply::Json(
            200,
            json!({"ResponseCode":"0", "ResponseDescription":"success"}),
        ),
        Reply::Json(200, status_ack()),
        Reply::Json(200, status_ack()),
    ])
    .await;
    let collection = server.collection();
    assert_eq!(server.count(), 0); // construction must not register URLs
    let ack = collection.submit_stk(&submission()).await.unwrap();
    assert_eq!(ack.checkout_request_id.as_str(), "checkout-1");
    let observation = collection.query_stk(&id("checkout-1")).await.unwrap();
    assert_eq!(observation.result_code.as_str(), "1032"); // a status observation, not settlement
    collection.register_c2b(&registration()).await.unwrap();
    let hooks = Hooks::new(&server);
    server.state.require_prepared.store(true, Ordering::SeqCst);
    server
        .status(true)
        .request(&status_request(), &hooks)
        .await
        .unwrap();
    server
        .status(false)
        .request(&status_request(), &hooks)
        .await
        .unwrap();
    assert_eq!(hooks.rows.lock().unwrap().len(), 4);
    let seen = server.state.seen.lock().unwrap();
    assert_eq!(
        seen.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(),
        vec![
            "/mpesa/stkpush/v1/processrequest",
            "/mpesa/stkpushquery/v1/query",
            "/mpesa/c2b/v1/registerurl",
            "/mpesa/transactionstatus/v1/query",
            "/mpesa/transactionstatus/v1/query"
        ]
    );
    for (i, request) in seen.iter().enumerate() {
        assert_eq!(request.headers["content-type"], "application/json");
        assert_eq!(
            request.headers["authorization"],
            if i == 4 {
                "Bearer disbursement-token"
            } else {
                "Bearer collection-token"
            }
        );
    }
    let password =
        base64::engine::general_purpose::STANDARD.encode("174379test-passkey20261020123456");
    assert_eq!(
        seen[0].body,
        json!({"BusinessShortCode":"174379", "Password":password, "Timestamp":"20261020123456", "TransactionType":"CustomerPayBillOnline", "Amount":123, "PartyA":"254712345678", "PartyB":"174379", "PhoneNumber":"254712345678", "CallBackURL":submission().callback_url.expose(), "AccountReference":"ABCD1234", "TransactionDesc":"deposit"})
    );
    assert_eq!(
        seen[1].body,
        json!({"BusinessShortCode":"174379", "Password":password, "Timestamp":"20261020123456", "CheckoutRequestID":"checkout-1"})
    );
    assert_eq!(
        seen[2].body,
        json!({"ShortCode":"600000", "ResponseType":"Cancelled", "ConfirmationURL":registration().confirmation_url.expose(), "ValidationURL":registration().validation_url.expose()})
    );
    let request = status_request();
    assert_eq!(
        seen[3].body,
        json!({"Initiator":request.initiator.expose(), "SecurityCredential":request.security_credential.expose(), "CommandID":"TransactionStatusQuery", "TransactionID":"QAB1234567", "PartyA":"600000", "IdentifierType":4, "ResultURL":request.result_url.expose(), "QueueTimeOutURL":request.timeout_url.expose(), "Remarks":"reconcile", "Occasion":"query"})
    );
    assert_eq!(seen[3].body, seen[4].body);
}
#[tokio::test]
async fn http_provider_and_malformed_errors_preserve_codes_status_and_correlation_without_replay() {
    let cases = vec![
        (
            Reply::Json(
                503,
                json!({"requestId":"body-request", "errorCode":"503.001", "errorMessage":"provider-body-secret"}),
            ),
            ErrorKind::Http,
            503,
            Some("503.001"),
        ),
        (
            Reply::Json(
                200,
                json!({"requestId":"body-request", "errorCode":"400.002", "errorMessage":"provider-body-secret"}),
            ),
            ErrorKind::Provider,
            200,
            Some("400.002"),
        ),
        (
            Reply::Json(
                200,
                json!({"ResponseCode":17, "OriginatorConversationID":"o-1", "ConversationID":"c-1"}),
            ),
            ErrorKind::Provider,
            200,
            Some("17"),
        ),
        (
            Reply::Raw(200, "invalid-json-provider-body-secret"),
            ErrorKind::InvalidResponse,
            200,
            None,
        ),
        (
            Reply::Raw(500, "html-provider-body-secret"),
            ErrorKind::Http,
            500,
            None,
        ),
        (
            Reply::Raw(200, "{\"ResponseCode\":\"0\",\"ResponseCode\":\"1\"}"),
            ErrorKind::InvalidResponse,
            200,
            None,
        ),
        (
            Reply::Json(200, json!({"ResponseCode":null})),
            ErrorKind::InvalidResponse,
            200,
            None,
        ),
        (
            Reply::Json(401, json!({"errorCode":"401.001"})),
            ErrorKind::Http,
            401,
            Some("401.001"),
        ),
        (Reply::Redirect, ErrorKind::Http, 307, None),
    ];
    for (reply, kind, status, code) in cases {
        let server = Server::new(vec![reply]).await;
        let error = server
            .collection()
            .submit_stk(&submission())
            .await
            .unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(error.outcome.http_status, Some(status));
        assert_eq!(error.outcome.provider_code.as_ref().map(Id::as_str), code);
        assert!(error.outcome.request_id.is_some());
        assert_eq!(server.count(), 1);
        assert_eq!(
            server.state.auth_calls.load(Ordering::SeqCst),
            if status == 401 { 2 } else { 1 }
        );
        assert!(!format!("{error:?} {error}").contains("provider-body-secret"));
        if code == Some("17") {
            assert_eq!(
                error.outcome.conversation_id.as_ref().unwrap().as_str(),
                "c-1"
            );
        }
        if code == Some("503.001") {
            assert_eq!(
                error.outcome.request_id.as_ref().unwrap().as_str(),
                "body-request"
            );
        }
    }
}
struct Runtime(Mutex<Vec<Duration>>);
impl QueryRuntime for Runtime {
    fn entropy(&self) -> u64 {
        123_456_789
    }
    async fn sleep(&self, delay: Duration) {
        self.0.lock().unwrap().push(delay);
    }
}
fn throttle(status: u16) -> Reply {
    Reply::Json(
        status,
        json!({"requestId":"throttle-id", "errorCode":"500.001.1001", "errorMessage":"Spike arrest violation"}),
    )
}
#[tokio::test]
async fn spike_arrest_has_bounded_query_only_exponential_jitter_and_recovers() {
    let policy =
        QueryRetryPolicy::new(2, Duration::from_millis(100), Duration::from_millis(150)).unwrap();
    assert_eq!(
        policy.delay(0, u64::MAX),
        Duration::from_nanos(u64::MAX % 100_000_001)
    );
    assert_eq!(
        policy.delay(1, u64::MAX),
        Duration::from_nanos(u64::MAX % 150_000_001)
    );
    assert_eq!(policy.delay(255, 150_000_000), Duration::from_millis(150));
    let runtime = Runtime(Mutex::new(Vec::new()));
    let server = Server::new(vec![
        throttle(429),
        throttle(200),
        Reply::Json(200, stk_query()),
    ])
    .await;
    server
        .collection()
        .query_stk_with_retry(&id("checkout-1"), policy, &runtime)
        .await
        .unwrap();
    assert_eq!(server.count(), 3);
    assert_eq!(
        *runtime.0.lock().unwrap(),
        vec![
            policy.delay(0, runtime.entropy()),
            policy.delay(1, runtime.entropy())
        ]
    );
    let server = Server::new(vec![throttle(500), throttle(500), throttle(500)]).await;
    let error = server
        .collection()
        .query_stk_with_retry(&id("checkout-1"), policy, &runtime)
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Throttled);
    assert_eq!(
        error.outcome.provider_code.unwrap().as_str(),
        "500.001.1001"
    );
    assert_eq!(server.count(), 3);
    let server = Server::new(vec![throttle(500)]).await;
    assert_eq!(
        server
            .collection()
            .submit_stk(&submission())
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Throttled
    );
    assert_eq!(server.count(), 1);
    let server = Server::new(vec![Reply::Json(503, json!({"errorCode":"503.001"}))]).await;
    assert_eq!(
        server
            .collection()
            .query_stk_with_retry(&id("checkout-1"), policy, &runtime)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Http
    );
    assert_eq!(server.count(), 1);
}
#[tokio::test]
async fn real_request_and_body_deadlines_keep_http_outcome_when_available() {
    for (reply, phase, status) in [
        (Reply::HeadersHang, Phase::Request, None),
        (Reply::BodyHang, Phase::Body, Some(200)),
    ] {
        let server = Server::new(vec![reply]).await;
        let error = server
            .collection_timeout(Duration::from_millis(150))
            .submit_stk(&submission())
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Timeout(phase));
        assert_eq!(error.outcome.http_status, status);
        assert_eq!(server.count(), 1);
        server.state.release.notify_waiters();
    }
}
#[tokio::test]
async fn real_tls_handshake_connect_deadline_is_distinct() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "https://{}/mpesa/stkpush/v1/processrequest",
        listener.local_addr().unwrap()
    )
    .parse()
    .unwrap();
    let server = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let error = ReqwestTransport::new(Duration::from_millis(100), Duration::from_secs(2))
        .unwrap()
        .post(HttpRequest {
            url,
            authorization: "Bearer test".parse().unwrap(),
            body: Secret::new("{}"),
        })
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::Timeout(Phase::Connect));
    assert_eq!(error.outcome.http_status, None);
    server.abort();
}
#[tokio::test]
async fn async_hooks_fail_closed_preserve_prepared_request_on_ambiguity_and_gate_lookup() {
    let server = Server::new(vec![Reply::HeadersHang]).await;
    let mut hooks = Hooks::new(&server);
    let client = server.status(false);
    hooks.fail_prepare = true;
    assert_eq!(
        client
            .request(&status_request(), &hooks)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Persistence
    );
    assert_eq!(server.count(), 0);
    hooks.fail_prepare = false;
    assert_eq!(
        client
            .request(&status_request(), &hooks)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Timeout(Phase::Request)
    );
    assert_eq!(hooks.rows.lock().unwrap().len(), 1);
    assert_eq!(server.count(), 1);
    let server = Server::new(vec![Reply::Json(200, status_ack())]).await;
    let mut hooks = Hooks::new(&server);
    hooks.fail_ack = true;
    let client = server.status(false);
    let error = client.request(&status_request(), &hooks).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Persistence);
    assert_eq!(
        error.outcome.originator_conversation_id.unwrap().as_str(),
        "originator-1"
    );
    assert_eq!(hooks.rows.lock().unwrap().len(), 1);
    let mut request = status_request();
    request.lookup = StatusLookup::OriginalConversation(id("original-1"));
    assert_eq!(
        client.request(&request, &hooks).await.unwrap_err().kind,
        ErrorKind::CapabilityDisabled
    );
    assert_eq!(server.count(), 1);
    let server = Server::new(vec![Reply::Json(200, status_ack())]).await;
    let mut client = server.status(false);
    client.enable_original_conversation_lookup();
    client
        .request(&request, &Hooks::new(&server))
        .await
        .unwrap();
    let seen = server.state.seen.lock().unwrap();
    assert_eq!(seen[0].body["OriginalConversationID"], "original-1");
    assert!(seen[0].body.get("OriginatorConversationID").is_none());
    assert!(seen[0].body.get("TransactionID").is_none());
}
#[tokio::test]
async fn input_amounts_and_callback_urls_fail_before_dispatch_and_query_ids_must_match() {
    let server = Server::new(vec![]).await;
    let client = server.collection();
    for amount in [
        Amount::new(0, Unit::KesMinor),
        Amount::new(101, Unit::KesMinor),
        Amount::new(100, Unit::Sat),
    ] {
        let mut request = submission();
        request.amount = amount;
        assert_eq!(
            client.submit_stk(&request).await.unwrap_err().kind,
            ErrorKind::Input
        );
    }
    let mut request = submission();
    request.callback_url = Secret::new("http://example.invalid/secret");
    assert_eq!(
        client.submit_stk(&request).await.unwrap_err().kind,
        ErrorKind::Input
    );
    assert_eq!(server.count(), 0);
    let server = Server::new(vec![Reply::Json(200, stk_query())]).await;
    assert_eq!(
        server
            .collection()
            .query_stk(&id("other-checkout"))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidResponse
    );
}
#[test]
fn status_result_timeout_dtos_and_exact_amounts_are_observations_not_acknowledgments() {
    let result = json!({"Result":{"ResultType":0,"ResultCode":0,"ResultDesc":"The service request is processed successfully.", "OriginatorConversationID":"originator-1", "ConversationID":"conversation-1", "TransactionID":"QAB1234567", "ResultParameters":{"ResultParameter":[{"Key":"ReceiptNo","Value":"QAB1234567"}, {"Key":"Amount","Value":"123.45"}]}}});
    let parsed = StatusResult::parse(&result.to_string()).unwrap();
    assert_eq!(parsed.result_code.as_str(), "0");
    assert_eq!(parsed.transaction_id.unwrap().as_str(), "QAB1234567");
    assert_eq!(
        parse_kes(&json!(parsed.parameters["Amount"].expose())).unwrap(),
        Amount::new(12345, Unit::KesMinor)
    );
    let timeout = json!({"Result":{"ResultCode":"1", "OriginatorConversationID":"originator-1", "ConversationID":"conversation-1", "ResultDesc":"timeout"}});
    assert_eq!(
        StatusTimeout::parse(&timeout.to_string())
            .unwrap()
            .result_code
            .as_str(),
        "1"
    );
    assert!(StatusResult::parse(&status_ack().to_string()).is_err());
    assert!(StatusResult::parse(r#"{"Result":{"ResultCode":0,"ResultCode":1}}"#).is_err());
    let mut contradiction = result.clone();
    contradiction["Result"]["TransactionID"] = json!("OTHER123456");
    assert!(StatusResult::parse(&contradiction.to_string()).is_err());
    let mut invalid_amount = result.clone();
    invalid_amount["Result"]["ResultParameters"]["ResultParameter"][1]["Value"] = json!("-1.00");
    assert!(StatusResult::parse(&invalid_amount.to_string()).is_err());
    assert_eq!(parsed.observed_amount.unwrap().value(), 12345);
    let mut duplicate = result.clone();
    duplicate["Result"]["ResultParameters"]["ResultParameter"]
        .as_array_mut()
        .unwrap()
        .push(json!({"Key":"Amount","Value":"999"}));
    assert!(StatusResult::parse(&duplicate.to_string()).is_err());
    for value in [
        json!("-1"),
        json!("0"),
        json!("1e2"),
        json!("1.001"),
        json!("1."),
        json!(" 1"),
        json!(true),
        json!(1.5),
        json!("18446744073709551615"),
        json!("184467440737095516.16"),
    ] {
        assert!(parse_kes(&value).is_err(), "{value}");
    }
    for value in [json!(123), json!("123"), json!("123.00")] {
        assert_eq!(parse_kes(&value).unwrap().value(), 12300);
    }
    assert_eq!(parse_kes(&json!("0.01")).unwrap().value(), 1);
    assert_eq!(
        parse_kes(&json!("184467440737095516.15")).unwrap().value(),
        u64::MAX
    );
}
#[test]
fn configuration_limits_and_debug_redaction() {
    assert!(ReqwestTransport::new(Duration::ZERO, Duration::from_secs(1)).is_err());
    assert!(QueryRetryPolicy::new(6, Duration::from_secs(1), Duration::from_secs(2)).is_err());
    assert!(QueryRetryPolicy::new(1, Duration::from_secs(2), Duration::from_secs(1)).is_err());
    for secret in [
        "254712345678",
        "callback-url-secret",
        "initiator-secret",
        "encrypted-credential-secret",
        "result-secret",
        "timeout-secret",
        "confirmation-secret",
        "validation-secret",
    ] {
        assert!(
            !format!(
                "{:?} {:?} {:?}",
                submission(),
                registration(),
                status_request()
            )
            .contains(secret)
        );
    }
}

#[tokio::test]
async fn every_endpoint_rejects_acknowledgment_errors_and_invalid_json() {
    for operation in 0..4 {
        for (reply, kind) in [
            (
                Reply::Json(
                    200,
                    json!({"ResponseCode":"9", "ConversationID":"correlation-9"}),
                ),
                ErrorKind::Provider,
            ),
            (Reply::Raw(200, "{broken-json"), ErrorKind::InvalidResponse),
            (
                Reply::Json(
                    400,
                    json!({"errorCode":"400.001.03", "requestId":"request-9"}),
                ),
                ErrorKind::Http,
            ),
        ] {
            let server = Server::new(vec![reply]).await;
            let client = server.collection();
            let result = match operation {
                0 => client.submit_stk(&submission()).await.map(|_| ()),
                1 => client.query_stk(&id("checkout-1")).await.map(|_| ()),
                2 => client.register_c2b(&registration()).await.map(|_| ()),
                _ => server
                    .status(false)
                    .request(&status_request(), &Hooks::new(&server))
                    .await
                    .map(|_| ()),
            };
            assert_eq!(result.unwrap_err().kind, kind);
            assert_eq!(server.count(), 1);
        }
    }
}

struct InspectTransport(ReqwestTransport);
impl HttpTransport for InspectTransport {
    async fn post(&self, request: HttpRequest) -> Result<daraja_core::transport::HttpResponse> {
        assert!(request.authorization.is_sensitive());
        assert!(!format!("{request:?}").contains("callback-url-secret"));
        self.0.post(request).await
    }
}
#[tokio::test]
async fn injected_transport_preserves_sensitive_headers_and_enforces_body_limit() {
    let server = Server::new(vec![Reply::Json(
        200,
        json!({"ResponseCode":"0", "padding":"x".repeat(65536)}),
    )])
    .await;
    let client = CollectionClient::new(
        InspectTransport(
            ReqwestTransport::new(Duration::from_secs(1), Duration::from_secs(2)).unwrap(),
        ),
        server.base.clone(),
        CollectionApp::new(server.config("collection")).unwrap(),
        FixedClock,
        StkConfig {
            shortcode: id("174379"),
            passkey: Secret::new("test-passkey"),
        },
    )
    .unwrap();
    let error = client.submit_stk(&submission()).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::BodyLimit);
    assert_eq!(error.outcome.http_status, Some(200));
    assert_eq!(
        error.outcome.request_id.unwrap().as_str(),
        "header-request-id"
    );
    assert_eq!(server.count(), 1);
}

#[path = "support/payout.rs"]
mod payout_contract;

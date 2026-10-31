use super::*;
use daraja_core::{
    Error, Timestamp,
    crypto::{Environment, SecurityCredentialKey},
    identity::RequestKey,
    outbound::{CdkPaymentState, DispatchConfig, MakePayment, OutboundPayments, OutboundState},
    payout::{Capabilities, PayoutClient, Remitter},
    repository::{Repository, RepositoryError},
    tariffs::{Band, Schedule, Tariffs},
    valuation::PaymentQuote,
    webhook::{self, Endpoint, Policy},
};
use sqlx::{Connection, SqliteConnection};

fn time() -> Timestamp {
    Timestamp(FixedClock.now().timestamp() as u64)
}
fn kes(v: u64) -> Amount {
    Amount::new(v, Unit::KesMinor)
}
fn make(key: &str, quote: &str) -> MakePayment {
    MakePayment {
        key: RequestKey {
            caller: id("mint"),
            operation: id("payout"),
            key: id(key),
        },
        quote_id: id(quote),
    }
}
async fn quote(repo: &Repository, name: &str, target: &str) {
    let schedule = |version| {
        vec![Schedule {
            version: id(version),
            effective_from: Timestamp(time().0 - 1),
            effective_until: Timestamp(time().0 + 600),
            bands: vec![Band {
                min_kes: 1,
                max_kes: 100000,
                fee: kes(100),
            }],
        }]
    };
    let tariffs = Tariffs::new(schedule("fixture-b2c"), schedule("fixture-b2b")).unwrap();
    let q = PaymentQuote::new(
        id(name),
        target,
        kes(12300),
        Unit::KesMinor,
        &tariffs,
        None,
        time(),
        Timestamp(time().0 + 300),
    )
    .unwrap();
    super::liquidity_fixture::fund(repo).await;
    repo.save_payment_quote(&q, time()).await.unwrap();
}
fn payout_client<T: HttpTransport>(transport: T, base: reqwest::Url) -> PayoutClient<T> {
    let key = SecurityCredentialKey::from_certificate(
        Environment::Sandbox,
        include_bytes!("../fixtures/rsa-cert.txt"),
    )
    .unwrap();
    let remitter = Remitter::new(
        Environment::Sandbox,
        &key,
        Secret::new("fixture-initiator"),
        &Secret::new("fixture-password"),
        id("600000"),
    )
    .unwrap();
    PayoutClient::new(
        transport,
        base.clone(),
        DisbursementApp::new(OAuthConfig {
            endpoint: base.join("disbursement").unwrap(),
            consumer_key: Secret::new("disbursement"),
            consumer_secret: Secret::new("secret"),
        })
        .unwrap(),
        remitter,
        100000,
    )
    .unwrap()
}
fn callback_secret(endpoint: Endpoint) -> String {
    format!(
        "{:064x}",
        Endpoint::ALL.iter().position(|e| e == &endpoint).unwrap() + 1
    )
}
fn config() -> DispatchConfig {
    DispatchConfig {
        b2c_result_url: Secret::new(format!(
            "https://example.invalid/callbacks/b2c/{}",
            callback_secret(Endpoint::B2c)
        )),
        b2c_timeout_url: Secret::new(format!(
            "https://example.invalid/callbacks/b2c/timeout/{}",
            callback_secret(Endpoint::B2cTimeout)
        )),
        b2b_result_url: Secret::new(format!(
            "https://example.invalid/callbacks/b2b/{}",
            callback_secret(Endpoint::B2b)
        )),
        b2b_timeout_url: Secret::new(format!(
            "https://example.invalid/callbacks/b2b/timeout/{}",
            callback_secret(Endpoint::B2bTimeout)
        )),
        first_check_seconds: 35,
        manual_after_seconds: 7200,
    }
}
fn transport() -> ReqwestTransport {
    ReqwestTransport::new(Duration::from_millis(100), Duration::from_millis(500)).unwrap()
}
struct BoundaryTransport {
    repository: Repository,
    inner: ReqwestTransport,
    crash: Option<String>,
    callback: Option<(reqwest::Url, Value)>,
    block_ack: Option<std::path::PathBuf>,
}
impl HttpTransport for BoundaryTransport {
    async fn post(&self, request: HttpRequest) -> Result<daraja_core::transport::HttpResponse> {
        let body: Value = serde_json::from_str(request.body.expose()).unwrap();
        let operation = id(body["OriginatorConversationID"].as_str().unwrap());
        let (payout, _) = self.repository.payout(&operation).await.unwrap();
        assert_eq!(payout.state(), OutboundState::Dispatching);
        assert_eq!(
            payout.dispatch().wire_body(),
            request.body.expose().as_bytes()
        );
        assert_eq!(payout.dispatch().reserved(), kes(12400));
        assert_eq!(payout.correlation().originator_conversation, operation);
        if self.crash.as_deref() == Some("before") {
            std::process::exit(73);
        }
        let result = self.inner.post(request).await;
        if let Some((url, callback)) = &self.callback {
            assert_eq!(
                reqwest::Client::new()
                    .post(url.clone())
                    .header("content-type", "application/json")
                    .body(callback.to_string())
                    .send()
                    .await
                    .unwrap()
                    .status(),
                200
            );
        }
        if self.callback.is_some() {
            assert!(
                self.repository
                    .payout_callback_ids(&operation)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        if self.crash.as_deref() == Some("after") {
            std::process::exit(73);
        }
        if let Some(path) = &self.block_ack {
            let mut db = SqliteConnection::connect(&format!("sqlite://{}", path.display()))
                .await
                .unwrap();
            sqlx::query("CREATE TRIGGER reject_ack BEFORE UPDATE ON outbound_payouts BEGIN SELECT RAISE(ABORT,'injected failure'); END").execute(&mut db).await.unwrap();
        }
        result
    }
}
fn boundary(repository: &Repository) -> BoundaryTransport {
    BoundaryTransport {
        repository: repository.clone(),
        inner: transport(),
        crash: None,
        callback: None,
        block_ack: None,
    }
}
fn service(repo: &Repository, server: &Server) -> OutboundPayments<BoundaryTransport, FixedClock> {
    OutboundPayments::new(
        repo.clone(),
        payout_client(boundary(repo), server.base.clone()),
        FixedClock,
        config(),
    )
    .unwrap()
}
fn assert_pending(p: &daraja_core::outbound::Payout) {
    assert_eq!(p.state().cdk_state(), CdkPaymentState::Pending);
}
async fn db(path: &std::path::Path) -> SqliteConnection {
    SqliteConnection::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap()
}
async fn count(db: &mut SqliteConnection, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn concurrent_callers_claim_once_and_payload_conflicts_cannot_send() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("payout.sqlite");
    let repo = Repository::open(&path).await.unwrap();
    quote(&repo, "quote", "0712345678").await;
    quote(&repo, "different", "0712345679").await;
    let server = Server::new(vec![Reply::Json(200, status_ack())]).await;
    let mut tasks = Vec::new();
    let barrier = Arc::new(tokio::sync::Barrier::new(12));
    for _ in 0..12 {
        // Independent connection pools and clients model separate service instances.
        let repo = Repository::open(&path).await.unwrap();
        let service = service(&repo, &server);
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service
                .make_payment(make("stable-key", "quote"))
                .await
                .unwrap()
        }));
    }
    let mut payouts = Vec::new();
    for task in tasks {
        payouts.push(task.await.unwrap());
    }
    for p in &payouts {
        assert_pending(p);
        assert_eq!(
            p.dispatch().operation_id(),
            payouts[0].dispatch().operation_id()
        );
    }
    assert_eq!(server.count(), 1);
    let service = service(&repo, &server);
    assert!(matches!(
        service.make_payment(make("stable-key", "different")).await,
        Err(daraja_core::outbound::DispatchError::Repository(
            RepositoryError::Domain(Error::IdempotencyConflict)
        ))
    ));
    // A new key cannot spend the same immutable quote a second time.
    assert!(
        service
            .make_payment(make("another-key", "quote"))
            .await
            .is_err()
    );
    let p = service
        .make_payment(make("stable-key", "quote"))
        .await
        .unwrap();
    assert_eq!(p.state(), OutboundState::Pending);
    let wire: Value = serde_json::from_slice(p.dispatch().wire_body()).unwrap();
    let uuid = wire["OriginatorConversationID"].as_str().unwrap();
    assert_eq!(uuid.len(), 36);
    assert_eq!(&uuid[14..15], "8");
    assert_ne!(
        p.correlation()
            .provider_originator
            .as_ref()
            .unwrap()
            .as_str(),
        uuid
    );
    let mut db = db(&path).await;
    for table in [
        "outbound_payouts",
        "payout_dispatches",
        "balance_reservations",
        "payout_dispatch_observations",
    ] {
        assert_eq!(count(&mut db, table).await, 1);
    }
    let outcome: String = sqlx::query_scalar("SELECT outcome FROM payout_dispatch_observations")
        .fetch_one(&mut db)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&outcome).unwrap()["http_status"],
        200
    );
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn ambiguous_provider_responses_and_duplicate_never_fail_or_replay() {
    for reply in [
        Reply::Json(
            200,
            json!({"errorCode":"synthetic-reject","errorMessage":"no guarantee"}),
        ),
        Reply::Json(200, json!({"ResponseCode":"42"})),
        Reply::Raw(200, "{broken"),
        Reply::Json(200, json!({"ResponseCode":"0"})),
        Reply::Json(500, json!({"errorCode":"synthetic-reject"})),
        Reply::Json(503, json!({"errorCode":"synthetic-reject"})),
        Reply::Json(400, json!({"errorCode":"500.002.1001"})),
        Reply::HeadersHang,
        Reply::BodyHang,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payout.sqlite");
        let repo = Repository::open(&path).await.unwrap();
        quote(&repo, "quote", "0712345678").await;
        let duplicate =
            matches!(&reply, Reply::Json(_, body) if body["errorCode"] == "500.002.1001");
        let server = Server::new(vec![reply]).await;
        let mut client = payout_client(boundary(&repo), server.base.clone());
        // A documented code is classified separately; even this synthetic policy
        // cannot turn 5xx or an observation without final evidence into Failed.
        client
            .configure_proven_rejections(
                daraja_core::payout::Command::BusinessPayment,
                vec![id("synthetic-reject")],
            )
            .unwrap();
        let svc = OutboundPayments::new(repo.clone(), client, FixedClock, config()).unwrap();
        let payout = svc.make_payment(make("key", "quote")).await.unwrap();
        assert_pending(&payout);
        assert_eq!(
            payout.state(),
            if duplicate {
                OutboundState::Pending
            } else {
                OutboundState::UnknownInFlight
            }
        );
        let frozen = payout.dispatch().clone();
        drop(svc);
        drop(repo);
        let repo = Repository::open(&path).await.unwrap();
        let replay = service(&repo, &server)
            .make_payment(make("key", "quote"))
            .await
            .unwrap();
        assert_pending(&replay);
        assert_eq!(replay.dispatch(), &frozen);
        assert_eq!(server.count(), 1);
        let observation: (String, String) =
            sqlx::query_as("SELECT disposition,outcome FROM payout_dispatch_observations")
                .fetch_one(&mut db(&path).await)
                .await
                .unwrap();
        let outcome: Value = serde_json::from_str(&observation.1).unwrap();
        if outcome["provider_code"] == "synthetic-reject" {
            assert_eq!(
                observation.0,
                if outcome["http_status"] == 200 {
                    "ProvenRejection"
                } else {
                    "Uncertain"
                }
            );
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM balance_reservations WHERE released_at IS NULL"
            )
            .fetch_one(&mut db(&path).await)
            .await
            .unwrap(),
            1
        );
    }
}

async fn ingress(repo: &Repository) -> (reqwest::Url, tokio::task::JoinHandle<()>) {
    let secrets = Endpoint::ALL
        .into_iter()
        .map(|e| (e, Secret::new(callback_secret(e))))
        .collect();
    let app = webhook::router(repo.clone(), Policy::new(id("600000"), secrets).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (base, task)
}
fn callback(endpoint: Endpoint) -> Value {
    if matches!(endpoint, Endpoint::B2cTimeout | Endpoint::B2bTimeout) {
        json!({"Result":{"ResultCode":1,"ResultDesc":"queue timeout","OriginatorConversationID":"originator-1","ConversationID":"conversation-1"}})
    } else {
        json!({"Result":{"ResultCode":0,"ResultDesc":"Success","OriginatorConversationID":"originator-1","ConversationID":"conversation-1","TransactionID":"ABC1234567"}})
    }
}
#[tokio::test]
async fn early_proxy_callbacks_are_staged_and_acknowledgment_correlates_both_rails() {
    for (target, endpoint) in [
        ("0712345678", Endpoint::B2c),
        ("TILL:123456", Endpoint::B2b),
        ("PAYBILL:123456:ABC", Endpoint::B2b),
        ("0712345678", Endpoint::B2cTimeout),
        ("TILL:123456", Endpoint::B2bTimeout),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payout.sqlite");
        let repo = Repository::open(&path).await.unwrap();
        quote(&repo, "quote", target).await;
        let server = Server::new(vec![Reply::Json(200, status_ack())]).await;
        let (base, task) = ingress(&repo).await;
        let url = base
            .join(
                &endpoint
                    .path()
                    .replace("{secret}", &callback_secret(endpoint)),
            )
            .unwrap();
        let mut transport = boundary(&repo);
        transport.callback = Some((url.clone(), callback(endpoint)));
        let mut client = payout_client(transport, server.base.clone());
        client
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
        let svc = OutboundPayments::new(repo.clone(), client, FixedClock, config()).unwrap();
        let p = svc.make_payment(make("key", "quote")).await.unwrap();
        assert_pending(&p);
        assert_eq!(
            p.state(),
            if matches!(endpoint, Endpoint::B2cTimeout | Endpoint::B2bTimeout) {
                OutboundState::UnknownInFlight
            } else {
                OutboundState::Pending
            }
        );
        assert_eq!(
            repo.payout_callback_ids(p.dispatch().operation_id())
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            reqwest::Client::new()
                .post(url)
                .header("content-type", "application/json")
                .body(callback(endpoint).to_string())
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            repo.payout_callback_ids(p.dispatch().operation_id())
                .await
                .unwrap()
                .len(),
            1
        );
        let (p, _) = repo.payout(p.dispatch().operation_id()).await.unwrap();
        assert_pending(&p);
        assert_eq!(server.count(), 1);
        task.abort();
    }
}

#[tokio::test]
async fn dispatch_crash_child() {
    let Ok(path) = std::env::var("DARAJA_STAGE12_DB") else {
        return;
    };
    let base = std::env::var("DARAJA_STAGE12_BASE")
        .unwrap()
        .parse()
        .unwrap();
    let repo = Repository::open(path).await.unwrap();
    let mut transport = boundary(&repo);
    transport.crash = Some(std::env::var("DARAJA_STAGE12_CRASH").unwrap());
    let svc =
        OutboundPayments::new(repo, payout_client(transport, base), FixedClock, config()).unwrap();
    let _ = svc.make_payment(make("key", "quote")).await;
    panic!("crash point not reached");
}
#[tokio::test]
async fn process_exits_before_send_and_after_send_never_authorize_another_transfer() {
    for (phase, expected_sends) in [("before", 0), ("after", 1)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payout.sqlite");
        let repo = Repository::open(&path).await.unwrap();
        quote(&repo, "quote", "0712345678").await;
        drop(repo);
        let server = Server::new(vec![Reply::Json(200, status_ack())]).await;
        let base = server.base.to_string();
        let child_path = path.clone();
        let status = tokio::task::spawn_blocking(move || {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "outbound_dispatch::dispatch_crash_child",
                    "--nocapture",
                ])
                .env("DARAJA_STAGE12_DB", child_path)
                .env("DARAJA_STAGE12_BASE", base)
                .env("DARAJA_STAGE12_CRASH", phase)
                .status()
                .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(status.code(), Some(73));
        assert_eq!(server.count(), expected_sends);
        let repo = Repository::open(&path).await.unwrap();
        assert_eq!(repo.recover_dispatching_payouts(time()).await.unwrap(), 1);
        assert_eq!(repo.recover_dispatching_payouts(time()).await.unwrap(), 0);
        let svc = service(&repo, &server);
        let p = svc.make_payment(make("key", "quote")).await.unwrap();
        assert_eq!(p.state(), OutboundState::UnknownInFlight);
        assert_pending(&p);
        assert_eq!(server.count(), expected_sends);
        assert!(
            repo.claim_job(Timestamp(time().0 + 35), 30)
                .await
                .unwrap()
                .is_some()
        );
        assert_pending(&svc.make_payment(make("key", "quote")).await.unwrap());
        assert_eq!(server.count(), expected_sends);
    }
}

#[tokio::test]
async fn ack_write_failure_rolls_back_ids_observation_and_preserves_early_callback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("payout.sqlite");
    let repo = Repository::open(&path).await.unwrap();
    quote(&repo, "quote", "0712345678").await;
    let server = Server::new(vec![Reply::Json(200, status_ack())]).await;
    let (base, task) = ingress(&repo).await;
    let mut transport = boundary(&repo);
    transport.callback = Some((
        base.join(
            &Endpoint::B2c
                .path()
                .replace("{secret}", &callback_secret(Endpoint::B2c)),
        )
        .unwrap(),
        callback(Endpoint::B2c),
    ));
    transport.block_ack = Some(path.clone());
    let svc = OutboundPayments::new(
        repo.clone(),
        payout_client(transport, server.base.clone()),
        FixedClock,
        config(),
    )
    .unwrap();
    assert!(svc.make_payment(make("key", "quote")).await.is_err());
    let mut db = db(&path).await;
    assert_eq!(count(&mut db, "callback_inbox").await, 1);
    assert_eq!(count(&mut db, "payout_dispatch_observations").await, 0);
    assert_eq!(count(&mut db, "payout_callback_links").await, 0);
    sqlx::query("DROP TRIGGER reject_ack")
        .execute(&mut db)
        .await
        .unwrap();
    repo.recover_dispatching_payouts(time()).await.unwrap();
    let p = service(&repo, &server)
        .make_payment(make("key", "quote"))
        .await
        .unwrap();
    assert_pending(&p);
    assert!(p.correlation().provider_originator.is_none());
    assert_eq!(server.count(), 1);
    task.abort();
}

struct LaterClock;
impl Clock for LaterClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        FixedClock.now() + chrono::Duration::seconds(1000)
    }
}
#[tokio::test]
async fn expired_replay_preserves_original_ciphertext_and_disabled_b2b_gate_prevents_first_send() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("payout.sqlite");
    let repo = Repository::open(&path).await.unwrap();
    quote(&repo, "quote", "0712345678").await;
    quote(&repo, "merchant", "TILL:123456").await;
    let server = Server::new(vec![Reply::Json(200, status_ack())]).await;
    assert!(
        service(&repo, &server)
            .make_payment(make("merchant", "merchant"))
            .await
            .is_err()
    );
    assert_eq!(server.count(), 0);
    assert_eq!(count(&mut db(&path).await, "outbound_payouts").await, 0);
    let p = service(&repo, &server)
        .make_payment(make("key", "quote"))
        .await
        .unwrap();
    let frozen = p.dispatch().clone();
    let mut config = config();
    config.b2c_result_url = Secret::new("invalid-reloaded-url");
    // A new client generates different randomized RSA ciphertext. Replay must not use it.
    let svc = OutboundPayments::new(
        repo,
        payout_client(transport(), server.base.clone()),
        LaterClock,
        config,
    )
    .unwrap();
    let replay = svc.make_payment(make("key", "quote")).await.unwrap();
    assert_eq!(replay.dispatch(), &frozen);
    assert_pending(&replay);
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn late_timeout_and_original_caller_callbacks_preserve_pending_and_require_rail_and_ids() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("payout.sqlite");
    let repo = Repository::open(&path).await.unwrap();
    quote(&repo, "quote", "0712345678").await;
    let server = Server::new(vec![Reply::Json(200, status_ack())]).await;
    let p = service(&repo, &server)
        .make_payment(make("key", "quote"))
        .await
        .unwrap();
    let (base, task) = ingress(&repo).await;
    for (endpoint, body, expected_links) in [
        (Endpoint::B2b, callback(Endpoint::B2b), 0),
        (
            Endpoint::B2c,
            {
                let mut c = callback(Endpoint::B2c);
                c["Result"]["ConversationID"] = json!("unrelated");
                c
            },
            0,
        ),
        (
            Endpoint::B2c,
            {
                let mut c = callback(Endpoint::B2c);
                c["Result"]["OriginatorConversationID"] =
                    json!(p.correlation().originator_conversation.as_str());
                c
            },
            1,
        ),
        (Endpoint::B2cTimeout, callback(Endpoint::B2cTimeout), 2),
    ] {
        let url = base
            .join(
                &endpoint
                    .path()
                    .replace("{secret}", &callback_secret(endpoint)),
            )
            .unwrap();
        assert_eq!(
            reqwest::Client::new()
                .post(url)
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            repo.payout_callback_ids(p.dispatch().operation_id())
                .await
                .unwrap()
                .len(),
            expected_links
        );
        assert_pending(&repo.payout(p.dispatch().operation_id()).await.unwrap().0);
    }
    assert_eq!(
        repo.payout(p.dispatch().operation_id())
            .await
            .unwrap()
            .0
            .state(),
        OutboundState::UnknownInFlight
    );
    assert_eq!(server.count(), 1);
    task.abort();
}

#[tokio::test]
async fn socket_disconnect_after_receiving_request_is_unknown_and_never_replayed() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base: reqwest::Url = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let sends = Arc::new(AtomicUsize::new(0));
    let seen = sends.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&bytes);
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|s| s.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if body.len() >= length {
                        break;
                    }
                }
            }
            let text = String::from_utf8(bytes).unwrap();
            if text.starts_with("GET /disbursement?") {
                let body = r#"{"access_token":"fixture-token","expires_in":"3600"}"#;
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            } else {
                assert!(text.starts_with("POST /mpesa/b2c/v3/paymentrequest "));
                seen.fetch_add(1, Ordering::SeqCst);
            }
            socket.shutdown().await.unwrap();
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("payout.sqlite");
    let repo = Repository::open(&path).await.unwrap();
    quote(&repo, "quote", "0712345678").await;
    let svc = OutboundPayments::new(repo, payout_client(transport(), base), FixedClock, config())
        .unwrap();
    let p = svc.make_payment(make("key", "quote")).await.unwrap();
    assert_eq!(p.state(), OutboundState::UnknownInFlight);
    assert_pending(&p);
    assert_eq!(
        svc.make_payment(make("key", "quote"))
            .await
            .unwrap()
            .dispatch(),
        p.dispatch()
    );
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    task.abort();
}

#[path = "outbound_reconcile.rs"]
mod reconcile_stage13;

#[path = "liquidity.rs"]
mod liquidity_stage14;

#[tokio::test]
async fn operator_refund_uses_one_send_saga_and_consumes_exception_only_after_paid() {
    use daraja_core::{
        identity::ReceiptId,
        inbound::{DepositAttestation, DepositRail, EvidenceSource, VerifiedDeposit},
        operations::{OperatorCredential, OperatorRole, OperatorService},
        outbound::{FinalOutcome, PayoutAttestation},
        repository::{PayoutChange, RefundAuthorization},
    };
    for reply in [
        Reply::Json(503, json!({"errorCode":"temporary"})),
        Reply::Json(200, status_ack()),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("refund.sqlite");
        let repo = Repository::open(&path).await.unwrap();
        super::liquidity_fixture::fund(&repo).await;
        repo.confirm(
            VerifiedDeposit::attest(DepositAttestation {
                source: EvidenceSource::ReviewedStatement(id("statement")),
                receipt: ReceiptId::new("REFUND1").unwrap(),
                quote_id: None,
                amount: kes(12400),
                shortcode: id("600000"),
                rail: DepositRail::C2b {
                    account_ref: id("UNKNOWN1"),
                },
                received_at: Timestamp(time().0 - 10),
                verified_at: time(),
                payer: None,
            })
            .unwrap(),
            None,
        )
        .await
        .unwrap();
        let token = Secret::new("synthetic-operator-token-0000000000000000");
        let operator = OperatorService::new(
            repo.clone(),
            vec![
                OperatorCredential::new(id("reviewer"), &token, OperatorRole::Reconciler).unwrap(),
            ],
        )
        .unwrap();
        let schedule = |v| {
            vec![Schedule {
                version: id(v),
                effective_from: Timestamp(time().0 - 1),
                effective_until: Timestamp(time().0 + 600),
                bands: vec![Band {
                    min_kes: 1,
                    max_kes: 100000,
                    fee: kes(100),
                }],
            }]
        };
        let tariffs = Tariffs::new(schedule("refund-b2c"), schedule("refund-b2b")).unwrap();
        let q = PaymentQuote::new(
            id("refund-quote"),
            "0712345678",
            kes(12300),
            Unit::KesMinor,
            &tariffs,
            None,
            time(),
            Timestamp(time().0 + 300),
        )
        .unwrap();
        let auth = RefundAuthorization {
            operator: id("spoofed"),
            owner: id("reviewed-owner"),
            destination: q.destination().clone(),
            evidence: id("reviewed-original-payer"),
            receipts: vec![ReceiptId::new("REFUND1").unwrap()],
        };
        let reserved = operator
            .request_refund(
                &token,
                &id("refund"),
                auth.clone(),
                q.tariff().clone(),
                time(),
            )
            .await
            .unwrap();
        assert_eq!(reserved.authorization.operator, id("reviewer"));
        let server = Server::new(vec![reply]).await;
        let payments = service(&repo, &server);
        assert!(
            operator
                .execute_refund(
                    &Secret::new("wrong"),
                    &id("refund"),
                    &id("case"),
                    &q,
                    &payments,
                    time()
                )
                .await
                .is_err()
        );
        let p = operator
            .execute_refund(&token, &id("refund"), &id("case"), &q, &payments, time())
            .await
            .unwrap();
        assert_pending(&p);
        assert_eq!(server.count(), 1);
        assert!(
            payments
                .make_payment(make("steal-refund", "refund-quote"))
                .await
                .is_err()
        );
        let again = operator
            .execute_refund(&token, &id("refund"), &id("case"), &q, &payments, time())
            .await
            .unwrap();
        assert_eq!(again.dispatch().operation_id(), p.dispatch().operation_id());
        assert_eq!(server.count(), 1);
        let reopened = Repository::open(&path).await.unwrap();
        let restarted = OperatorService::new(
            reopened.clone(),
            vec![
                OperatorCredential::new(id("reviewer"), &token, OperatorRole::Reconciler).unwrap(),
            ],
        )
        .unwrap();
        let resumed_payments = service(&reopened, &server);
        let resumed = restarted
            .execute_refund(
                &token,
                &id("refund"),
                &id("case"),
                &q,
                &resumed_payments,
                time(),
            )
            .await
            .unwrap();
        assert_eq!(
            resumed.dispatch().operation_id(),
            p.dispatch().operation_id()
        );
        assert_eq!(server.count(), 1);
        let mut raw = db(&path).await;
        let unresolved: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM unreconciled_deposits WHERE resolved_at IS NULL",
        )
        .fetch_one(&mut raw)
        .await
        .unwrap();
        assert_eq!(unresolved, 1);
        assert!(
            operator
                .request_refund(
                    &token,
                    &id("second-refund"),
                    auth,
                    q.tariff().clone(),
                    time()
                )
                .await
                .is_err()
        );
        let (_, version) = repo.payout(p.dispatch().operation_id()).await.unwrap();
        repo.change_payout(
            p.dispatch().operation_id(),
            version,
            PayoutChange::Resolve(PayoutAttestation {
                source: EvidenceSource::ReviewedStatement(id("verified-paid-record")),
                operation_id: p.dispatch().operation_id().clone(),
                payload_hash: p.dispatch().hash(),
                amount: q.principal(),
                destination: q.destination().clone(),
                outcome: FinalOutcome::Paid {
                    receipt: ReceiptId::new("ACTUALREFUND").unwrap(),
                },
            }),
            time(),
        )
        .await
        .unwrap();
        assert_eq!(repo.refund_status(&id("refund")).await.unwrap(), "Paid");
        let unresolved: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM unreconciled_deposits WHERE resolved_at IS NULL",
        )
        .fetch_one(&mut raw)
        .await
        .unwrap();
        assert_eq!(unresolved, 0);
        assert_eq!(count(&mut raw, "settlement_events").await, 0);
        assert_eq!(server.count(), 1);
    }
}

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use daraja_core::{
    Error, Timestamp,
    amount::{Amount, TariffSnapshot, Unit},
    auth::{CollectionApp, OAuthConfig},
    checkout::*,
    collection::{CollectionClient, StkConfig},
    crypto::{Clock, Secret},
    identity::{Id, ReceiptId, RequestKey},
    inbound::{
        AttemptState, DepositAttestation, DepositRail, DepositState, EvidenceSource, InboundState,
        VerifiedDeposit,
    },
    repository::{Repository, RepositoryError, StkChange},
    transport::ReqwestTransport,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

fn id(s: &str) -> Id {
    Id::new(s).unwrap()
}
fn kes(n: u64) -> Amount {
    Amount::new(n, Unit::KesMinor)
}
#[derive(Clone)]
struct TestClock(Arc<AtomicI64>);
impl Clock for TestClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(self.0.load(Ordering::SeqCst), 0).unwrap()
    }
}
#[derive(Clone, Default)]
struct Identities {
    sequence: Arc<AtomicUsize>,
    refs: Arc<Mutex<VecDeque<String>>>,
}
impl CheckoutIdentity for Identities {
    fn id(&self) -> Result<Id> {
        Ok(id(&format!(
            "local-{}",
            self.sequence.fetch_add(1, Ordering::SeqCst)
        )))
    }
    fn account_ref(&self) -> Result<Id> {
        if let Some(s) = self.refs.lock().unwrap().pop_front() {
            return Ok(id(&s));
        }
        Ok(id(&format!(
            "R{:07}",
            self.sequence.fetch_add(1, Ordering::SeqCst)
        )))
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Success,
    HttpFailure,
    Reject,
    Hang,
}
struct Provider {
    db: std::path::PathBuf,
    mode: Mutex<Mode>,
    seen: Mutex<Vec<Value>>,
    queries: AtomicUsize,
    query_code: Mutex<String>,
    gate: Semaphore,
}
async fn token() -> Json<Value> {
    Json(json!({"access_token":"synthetic-token", "expires_in":3600}))
}
async fn submit(
    State(p): State<Arc<Provider>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    use sqlx::Connection;
    let mut conn = sqlx::SqliteConnection::connect(p.db.to_str().unwrap())
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stk_attempts a JOIN inbound_quotes q ON q.id=a.quote_id WHERE q.account_ref=?")
        .bind(body["AccountReference"].as_str().unwrap()).fetch_one(&mut conn).await.unwrap();
    assert!(count > 0, "attempt must be committed before collection I/O");
    let number = {
        let mut seen = p.seen.lock().unwrap();
        seen.push(body);
        seen.len()
    };
    let mode = *p.mode.lock().unwrap();
    if matches!(mode, Mode::Hang) {
        let _permit = p.gate.acquire().await.unwrap();
    }
    match mode {
        Mode::HttpFailure => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({}))),
        Mode::Reject => (
            StatusCode::OK,
            Json(
                json!({"ResponseCode":"1", "MerchantRequestID":"rejected", "CheckoutRequestID":"rejected"}),
            ),
        ),
        _ => (
            StatusCode::OK,
            Json(
                json!({"ResponseCode":"0", "MerchantRequestID":format!("merchant-{number}"), "CheckoutRequestID":format!("checkout-{number}")}),
            ),
        ),
    }
}
async fn query(State(p): State<Arc<Provider>>, Json(body): Json<Value>) -> Json<Value> {
    p.queries.fetch_add(1, Ordering::SeqCst);
    Json(
        json!({"ResponseCode":"0", "ResultCode":p.query_code.lock().unwrap().clone(), "MerchantRequestID":body["CheckoutRequestID"].as_str().unwrap().replace("checkout-", "merchant-"), "CheckoutRequestID":body["CheckoutRequestID"]}),
    )
}
type Client = CollectionClient<ReqwestTransport, TestClock>;
type Service = CheckoutService<Client, TestClock, Identities>;
struct Fixture {
    dir: tempfile::TempDir,
    repo: Repository,
    clock: TestClock,
    identities: Identities,
    provider: Arc<Provider>,
    client: Arc<Client>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new(mode: Mode) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("checkout.sqlite");
        let repo = Repository::open(&db).await.unwrap();
        let provider = Arc::new(Provider {
            db,
            mode: Mutex::new(mode),
            seen: Mutex::new(vec![]),
            queries: AtomicUsize::new(0),
            query_code: Mutex::new("1032".into()),
            gate: Semaphore::new(0),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base =
            reqwest::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let router = Router::new()
            .route("/oauth", get(token))
            .route("/mpesa/stkpush/v1/processrequest", post(submit))
            .route("/mpesa/stkpushquery/v1/query", post(query))
            .with_state(provider.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let clock = TestClock(Arc::new(AtomicI64::new(100)));
        let client = Arc::new(
            CollectionClient::new(
                ReqwestTransport::new(Duration::from_millis(100), Duration::from_millis(400))
                    .unwrap(),
                base.clone(),
                CollectionApp::new(OAuthConfig {
                    endpoint: base.join("oauth").unwrap(),
                    consumer_key: Secret::new("synthetic"),
                    consumer_secret: Secret::new("synthetic"),
                })
                .unwrap(),
                clock.clone(),
                StkConfig {
                    shortcode: id("174379"),
                    passkey: Secret::new("synthetic"),
                },
            )
            .unwrap(),
        );
        Self {
            dir,
            repo,
            clock,
            identities: Identities::default(),
            provider,
            client,
            task,
        }
    }
    fn config(&self) -> CheckoutConfig {
        CheckoutConfig {
            shortcode: id("174379"),
            ttl_seconds: 300,
            tariff: TariffSnapshot {
                version: id("operator-v1"),
                fee: kes(0),
            },
            settlement_hold_seconds: 0,
            stk_reconciliation: StkReconciliationPolicy::verified_failure_codes([id("1032")])
                .unwrap(),
            callback_url: Secret::new("https://callback.invalid/callbacks/stk/synthetic-secret"),
        }
    }
    fn service(&self) -> Service {
        self.service_with(self.repo.clone())
    }
    fn service_with(&self, repo: Repository) -> Service {
        CheckoutService::new(
            repo,
            self.client.clone(),
            Arc::new(self.clock.clone()),
            self.identities.clone(),
            self.config(),
        )
        .unwrap()
    }
    fn count(&self) -> usize {
        self.provider.seen.lock().unwrap().len()
    }
    async fn wait(&self, q: &Id, state: AttemptState) -> Checkout {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let c = self.service().checkout(q).await.unwrap();
                if c.stk_status == Some(state) {
                    return c;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap()
    }
    fn at(&self, seconds: i64) {
        self.clock.0.store(seconds, Ordering::SeqCst);
    }
}
fn key(operation: &str, k: &str) -> RequestKey {
    RequestKey {
        caller: id("caller"),
        operation: id(operation),
        key: id(k),
    }
}
fn request(k: &str, phone: bool) -> CreateCheckout {
    CreateCheckout {
        key: key("inbound-create", k),
        amount: kes(10000),
        phone: phone.then(|| Secret::new("254700000001")),
    }
}

#[tokio::test]
async fn manual_only_is_durable_exact_and_sat_is_unavailable() {
    let f = Fixture::new(Mode::Success).await;
    let s = f.service();
    let c = s.create(request("manual", false)).await.unwrap();
    assert_eq!(c.payment_state, InboundState::Issued);
    assert_eq!(c.attempt_id, None);
    assert_eq!(c.retry_after, None);
    assert_eq!(c.fallback.whole_kes, 100);
    assert_eq!(c.fallback.amount, kes(10000));
    assert_eq!(c.fallback.paybill, id("174379"));
    assert_eq!(c.fallback.account_ref.as_str().len(), 8);
    assert_eq!(c.expires_at, Timestamp(400));
    assert_eq!(f.count(), 0);
    let r = Repository::open(f.dir.path().join("checkout.sqlite"))
        .await
        .unwrap();
    assert_eq!(
        f.service_with(r)
            .create(request("manual", false))
            .await
            .unwrap(),
        c
    );
    let mut sat = request("sat", false);
    sat.amount = Amount::new(100, Unit::Sat);
    assert!(matches!(
        s.create(sat).await,
        Err(CheckoutError::UnsupportedUnit)
    ));
    let mut fractional = request("fraction", false);
    fractional.amount = kes(10001);
    assert!(matches!(
        s.create(fractional).await,
        Err(CheckoutError::Domain(Error::InvalidAmount))
    ));
    let mut conflict = request("manual", true);
    conflict.amount = kes(20000);
    assert!(matches!(
        s.create(conflict).await,
        Err(CheckoutError::Repository(RepositoryError::Domain(
            Error::IdempotencyConflict
        )))
    ));
}

#[tokio::test]
async fn concurrent_creation_dispatches_once_and_collision_retries_do_not_replace_bindings() {
    let f = Fixture::new(Mode::Success).await;
    f.identities.refs.lock().unwrap().extend([
        "COLLIDE1".into(),
        "COLLIDE1".into(),
        "UNIQUE02".into(),
    ]);
    let first = f.service().create(request("first", false)).await.unwrap();
    let s = Arc::new(f.service());
    let mut tasks = vec![];
    for _ in 0..12 {
        let s = s.clone();
        tasks.push(tokio::spawn(async move {
            s.create(request("race", true)).await.unwrap()
        }));
    }
    let mut quotes = vec![];
    for t in tasks {
        quotes.push(t.await.unwrap());
    }
    for c in &quotes {
        assert_eq!(c.quote_id, quotes[0].quote_id);
        assert_eq!(c.attempt_id, quotes[0].attempt_id);
    }
    assert_ne!(quotes[0].fallback.account_ref, first.fallback.account_ref);
    let sent = f.wait(&quotes[0].quote_id, AttemptState::Sent).await;
    assert_eq!(sent.checkout_request_id, Some(id("checkout-1")));
    assert_eq!(
        f.repo
            .quote(&sent.quote_id)
            .await
            .unwrap()
            .0
            .attempts()
            .len(),
        1
    );
    assert_eq!(f.count(), 1);
    assert_eq!(f.provider.seen.lock().unwrap()[0]["Amount"], 100);
}

#[tokio::test]
async fn immediate_fallback_survives_hanging_dispatch_and_secondary_payer_has_no_phone_binding() {
    let f = Fixture::new(Mode::Hang).await;
    let s = f.service();
    let c = tokio::time::timeout(Duration::from_millis(200), s.create(request("hang", true)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.stk_status, Some(AttemptState::Dispatching));
    assert_eq!(c.fallback.whole_kes, 100);
    assert_eq!(c.retry_after, Some(Timestamp(135)));
    let (_, a) = f
        .repo
        .attempt(c.attempt_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(a.next_poll_at(), c.retry_after.unwrap());
    f.wait(&c.quote_id, AttemptState::Unknown).await;
    assert_eq!(
        s.create(request("hang", true)).await.unwrap().quote_id,
        c.quote_id
    );
    assert_eq!(f.count(), 1);
    f.at(135);
    assert!(matches!(
        s.resend(
            &c.quote_id,
            key("inbound-resend", "unknown"),
            Secret::new("254700000002")
        )
        .await,
        Err(CheckoutError::ReconciliationRequired)
    ));
    use daraja_core::webhook::{Endpoint, Policy, router};
    let secrets = Endpoint::ALL
        .into_iter()
        .enumerate()
        .map(|(i, e)| (e, Secret::new(format!("{:064x}", i + 1))))
        .collect();
    let policy = Policy::new(c.fallback.paybill.clone(), secrets).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let ingress = router(f.repo.clone(), policy);
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            ingress.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let secret = format!("{:064x}", 3);
    let response = reqwest::Client::new().post(format!("http://{address}/callbacks/c2b/confirmation/{secret}"))
        .header("content-type", "application/json")
        .body(json!({"TransactionType":"Pay Bill", "TransID":"SECONDARY1", "TransTime":"20261020123456", "TransAmount":"100.00", "BusinessShortCode":c.fallback.paybill.as_str(), "BillRefNumber":c.fallback.account_ref.as_str(), "MSISDN":"254700000002"}).to_string())
        .send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        f.repo.quote(&c.quote_id).await.unwrap().0.state(),
        InboundState::Issued
    );
    task.abort();
    // Independently reviewed synthetic evidence, separate from that untrusted callback.
    let verified = VerifiedDeposit::attest(DepositAttestation {
        payer: None,
        source: EvidenceSource::ReviewedStatement(id("synthetic-secondary-payer-statement")),
        receipt: ReceiptId::new("SECONDARY1").unwrap(),
        quote_id: Some(c.quote_id.clone()),
        amount: c.fallback.amount,
        shortcode: c.fallback.paybill.clone(),
        rail: DepositRail::C2b {
            account_ref: c.fallback.account_ref.clone(),
        },
        received_at: Timestamp(120),
        verified_at: Timestamp(125),
    })
    .unwrap();
    assert_eq!(
        f.repo.confirm(verified, None).await.unwrap(),
        DepositState::Verified
    );
    let (_, version) = f.repo.quote(&c.quote_id).await.unwrap();
    f.repo
        .settle(&c.quote_id, version, Timestamp(130))
        .await
        .unwrap();
    assert_eq!(
        s.checkout(&c.quote_id).await.unwrap().payment_state,
        InboundState::Paid
    );
    assert_eq!(f.repo.events_after(0, 10).await.unwrap().len(), 1);
    f.provider.gate.add_permits(1);
}

#[tokio::test]
async fn provider_rejection_and_503_are_unresolved_and_never_replayed() {
    for mode in [Mode::HttpFailure, Mode::Reject] {
        let f = Fixture::new(mode).await;
        let c = f.service().create(request("failure", true)).await.unwrap();
        let failed = f.wait(&c.quote_id, AttemptState::Unknown).await;
        assert_eq!(failed.payment_state, InboundState::Issued);
        assert_eq!(failed.fallback, c.fallback);
        f.at(140);
        assert!(matches!(
            f.service()
                .resend(
                    &c.quote_id,
                    key("inbound-resend", "retry"),
                    Secret::new("254700000001")
                )
                .await,
            Err(CheckoutError::ReconciliationRequired)
        ));
        f.service().create(request("failure", true)).await.unwrap();
        assert_eq!(f.count(), 1);
    }
}

#[tokio::test]
async fn cooldown_survives_reopen_resend_reconciles_and_retains_old_attempt() {
    let f = Fixture::new(Mode::Success).await;
    let c = f.service().create(request("sent", true)).await.unwrap();
    let sent = f.wait(&c.quote_id, AttemptState::Sent).await;
    f.at(134);
    f.repo.close().await;
    let reopened = Repository::open(f.dir.path().join("checkout.sqlite"))
        .await
        .unwrap();
    let s = f.service_with(reopened.clone());
    assert!(matches!(
        s.resend(
            &c.quote_id,
            key("inbound-resend", "resend"),
            Secret::new("254700000002")
        )
        .await,
        Err(CheckoutError::Cooldown {
            retry_after: Timestamp(135)
        })
    ));
    assert_eq!(f.provider.queries.load(Ordering::SeqCst), 0);
    f.at(135);
    let resent = s
        .resend(
            &c.quote_id,
            key("inbound-resend", "resend"),
            Secret::new("254700000002"),
        )
        .await
        .unwrap();
    assert_eq!(resent.quote_id, c.quote_id);
    assert_eq!(resent.retry_after, Some(Timestamp(170)));
    assert_ne!(resent.attempt_id, c.attempt_id);
    tokio::time::timeout(Duration::from_secs(4), async {
        while s.checkout(&c.quote_id).await.unwrap().stk_status != Some(AttemptState::Sent) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let (q, _) = reopened.quote(&c.quote_id).await.unwrap();
    assert_eq!(q.attempts().len(), 2);
    assert_eq!(
        q.attempts()[0].checkout_request(),
        sent.checkout_request_id.as_ref()
    );
    assert_eq!(q.attempts()[0].state(), AttemptState::Failed);
    assert_eq!(f.provider.queries.load(Ordering::SeqCst), 1);
    assert_eq!(f.count(), 2);
    assert_eq!(
        f.provider.seen.lock().unwrap()[1]["PhoneNumber"],
        "254700000002"
    );
    s.resend(
        &c.quote_id,
        key("inbound-resend", "resend"),
        Secret::new("254700000002"),
    )
    .await
    .unwrap();
    assert_eq!(f.count(), 2);
    assert!(matches!(
        s.resend(
            &c.quote_id,
            key("inbound-resend", "resend"),
            Secret::new("254700000003")
        )
        .await,
        Err(CheckoutError::Repository(RepositoryError::Domain(
            Error::IdempotencyConflict
        )))
    ));
    f.at(170);
    s.resend(
        &c.quote_id,
        key("inbound-resend", "third"),
        Secret::new("254700000003"),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        while s.checkout(&c.quote_id).await.unwrap().stk_status != Some(AttemptState::Sent) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        reopened
            .quote(&c.quote_id)
            .await
            .unwrap()
            .0
            .attempts()
            .len(),
        3
    );
    assert_eq!(f.count(), 3);
}

#[tokio::test]
async fn completed_or_unresolved_prior_attempt_blocks_another_prompt() {
    for code in ["0", "9999"] {
        let f = Fixture::new(Mode::Success).await;
        let c = f.service().create(request("sent", true)).await.unwrap();
        f.wait(&c.quote_id, AttemptState::Sent).await;
        *f.provider.query_code.lock().unwrap() = code.into();
        f.at(135);
        assert!(matches!(
            f.service()
                .resend(
                    &c.quote_id,
                    key("inbound-resend", "resend"),
                    Secret::new("254700000002")
                )
                .await,
            Err(CheckoutError::ReconciliationRequired)
        ));
        assert_eq!(f.count(), 1);
        assert_ne!(
            f.service()
                .checkout(&c.quote_id)
                .await
                .unwrap()
                .payment_state,
            InboundState::Paid
        );
    }
}

#[tokio::test]
async fn older_attempt_receipts_cannot_create_second_settlement_after_resend() {
    let f = Fixture::new(Mode::Success).await;
    let c = f.service().create(request("sent", true)).await.unwrap();
    f.wait(&c.quote_id, AttemptState::Sent).await;
    f.at(135);
    f.service()
        .resend(
            &c.quote_id,
            key("inbound-resend", "new"),
            Secret::new("254700000002"),
        )
        .await
        .unwrap();
    f.wait(&c.quote_id, AttemptState::Sent).await;
    for (index, checkout) in ["checkout-2", "checkout-1"].iter().enumerate() {
        let verified = VerifiedDeposit::attest(DepositAttestation {
            payer: None,
            source: EvidenceSource::ReviewedStatement(id("synthetic-statement")),
            receipt: ReceiptId::new(&format!("RECEIPT{index}")).unwrap(),
            quote_id: Some(c.quote_id.clone()),
            amount: kes(10000),
            shortcode: c.fallback.paybill.clone(),
            rail: DepositRail::Stk {
                checkout_request: id(checkout),
            },
            received_at: Timestamp(140),
            verified_at: Timestamp(145),
        })
        .unwrap();
        let disposition = f.repo.confirm(verified, None).await.unwrap();
        let (_, version) = f.repo.quote(&c.quote_id).await.unwrap();
        if index == 0 {
            assert_eq!(disposition, DepositState::Verified);
            f.repo
                .settle(&c.quote_id, version, Timestamp(150))
                .await
                .unwrap();
        } else {
            assert_eq!(disposition, DepositState::OverpaidHeld);
            assert!(
                f.repo
                    .settle(&c.quote_id, version, Timestamp(150))
                    .await
                    .is_err()
            );
        }
    }
    assert_eq!(f.repo.events_after(0, 100).await.unwrap().len(), 1);
}

#[tokio::test]
async fn unavailable_stk_preserves_fallback_and_failed_attempt_without_network() {
    let f = Fixture::new(Mode::Success).await;
    let service = CheckoutService::new(
        f.repo.clone(),
        Arc::new(UnavailableCollection {
            shortcode: id("174379"),
        }),
        Arc::new(f.clock.clone()),
        f.identities.clone(),
        f.config(),
    )
    .unwrap();
    let c = service.create(request("unavailable", true)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while service.checkout(&c.quote_id).await.unwrap().stk_status != Some(AttemptState::Failed)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let c = service.checkout(&c.quote_id).await.unwrap();
    assert_eq!(c.fallback.whole_kes, 100);
    assert_eq!(c.payment_state, InboundState::Issued);
    assert_eq!(c.retry_after, Some(Timestamp(135)));
    assert_eq!(f.count(), 0);
    assert_eq!(
        service.create(request("unavailable", true)).await.unwrap(),
        c
    );
}

#[tokio::test]
async fn restart_with_prepared_attempt_never_automatically_dispatches() {
    let f = Fixture::new(Mode::Success).await;
    let c = f.service().create(request("manual", false)).await.unwrap();
    let (_, version) = f.repo.quote(&c.quote_id).await.unwrap();
    f.repo
        .change_stk(
            &c.quote_id,
            version,
            StkChange::Start(
                daraja_core::inbound::StkAttempt::new(
                    id("prepared-before-stop"),
                    id("prepared-request"),
                    Timestamp(100),
                )
                .unwrap(),
            ),
            Timestamp(100),
        )
        .await
        .unwrap();
    f.repo.close().await;
    let reopened = Repository::open(f.dir.path().join("checkout.sqlite"))
        .await
        .unwrap();
    let s = f.service_with(reopened);
    let c = s.create(request("manual", false)).await.unwrap();
    assert_eq!(c.stk_status, Some(AttemptState::Dispatching));
    assert_eq!(c.retry_after, Some(Timestamp(135)));
    f.at(134);
    assert!(matches!(
        s.resend(
            &c.quote_id,
            key("inbound-resend", "retry"),
            Secret::new("254700000002")
        )
        .await,
        Err(CheckoutError::Cooldown { .. })
    ));
    f.at(135);
    assert!(matches!(
        s.resend(
            &c.quote_id,
            key("inbound-resend", "retry"),
            Secret::new("254700000002")
        )
        .await,
        Err(CheckoutError::ReconciliationRequired)
    ));
    assert_eq!(f.count(), 0);
}

#[tokio::test]
async fn default_policy_blocks_unverified_failure_codes_and_resends_race_safely() {
    let f = Fixture::new(Mode::Success).await;
    let c = f.service().create(request("sent", true)).await.unwrap();
    f.wait(&c.quote_id, AttemptState::Sent).await;
    f.at(135);
    let mut config = f.config();
    config.stk_reconciliation = StkReconciliationPolicy::default();
    let s = CheckoutService::new(
        f.repo.clone(),
        f.client.clone(),
        Arc::new(f.clock.clone()),
        f.identities.clone(),
        config,
    )
    .unwrap();
    assert!(matches!(
        s.resend(
            &c.quote_id,
            key("inbound-resend", "disabled"),
            Secret::new("254700000002")
        )
        .await,
        Err(CheckoutError::ReconciliationRequired)
    ));
    let s = f.service();
    let (a, b) = tokio::join!(
        s.resend(
            &c.quote_id,
            key("inbound-resend", "race-a"),
            Secret::new("254700000002")
        ),
        s.resend(
            &c.quote_id,
            key("inbound-resend", "race-b"),
            Secret::new("254700000003")
        )
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    f.wait(&c.quote_id, AttemptState::Sent).await;
    assert_eq!(
        f.repo.quote(&c.quote_id).await.unwrap().0.attempts().len(),
        2
    );
    assert_eq!(f.count(), 2);
    f.at(400);
    assert!(matches!(
        s.resend(
            &c.quote_id,
            key("inbound-resend", "expired"),
            Secret::new("254700000002")
        )
        .await,
        Err(CheckoutError::Domain(Error::QuoteExpired))
    ));
}

#[tokio::test]
async fn checkout_preparation_failure_rolls_back_quote_binding_and_attempt() {
    use sqlx::Connection;
    let f = Fixture::new(Mode::Success).await;
    let mut db = sqlx::SqliteConnection::connect(f.provider.db.to_str().unwrap())
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_checkout_binding BEFORE INSERT ON request_bindings BEGIN SELECT RAISE(ABORT,'synthetic failure'); END").execute(&mut db).await.unwrap();
    assert!(f.service().create(request("rollback", true)).await.is_err());
    for table in [
        "inbound_quotes",
        "request_bindings",
        "stk_attempts",
        "durable_jobs",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&mut db)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    assert_eq!(f.count(), 0);
    sqlx::query("DROP TRIGGER fail_checkout_binding")
        .execute(&mut db)
        .await
        .unwrap();
    let c = f.service().create(request("rollback", true)).await.unwrap();
    f.wait(&c.quote_id, AttemptState::Sent).await;
    assert_eq!(f.count(), 1);
}

#[tokio::test]
async fn acknowledgment_after_manual_settlement_retains_ids_and_holds_late_stk_payment() {
    let f = Fixture::new(Mode::Hang).await;
    let c = f.service().create(request("late-ack", true)).await.unwrap();
    tokio::time::timeout(Duration::from_millis(200), async {
        while f.count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let deposit = |receipt: &str, rail: DepositRail| {
        VerifiedDeposit::attest(DepositAttestation {
            payer: None,
            source: EvidenceSource::ReviewedStatement(id("synthetic-statement")),
            receipt: ReceiptId::new(receipt).unwrap(),
            quote_id: Some(c.quote_id.clone()),
            amount: c.fallback.amount,
            shortcode: c.fallback.paybill.clone(),
            rail,
            received_at: Timestamp(120),
            verified_at: Timestamp(125),
        })
        .unwrap()
    };
    f.repo
        .confirm(
            deposit(
                "MANUALFIRST",
                DepositRail::C2b {
                    account_ref: c.fallback.account_ref.clone(),
                },
            ),
            Some(Timestamp(130)),
        )
        .await
        .unwrap();
    f.provider.gate.add_permits(1);
    let recorded = f.wait(&c.quote_id, AttemptState::Sent).await;
    assert_eq!(recorded.payment_state, InboundState::Paid);
    assert_eq!(recorded.checkout_request_id, Some(id("checkout-1")));
    assert_eq!(
        f.repo
            .confirm(
                deposit(
                    "STKLATE",
                    DepositRail::Stk {
                        checkout_request: id("checkout-1")
                    }
                ),
                None
            )
            .await
            .unwrap(),
        DepositState::OverpaidHeld
    );
    let (q, version) = f.repo.quote(&c.quote_id).await.unwrap();
    assert_eq!(q.state(), InboundState::Paid);
    assert!(
        f.repo
            .settle(&c.quote_id, version, Timestamp(150))
            .await
            .is_err()
    );
    assert_eq!(f.repo.events_after(0, 10).await.unwrap().len(), 1);
}

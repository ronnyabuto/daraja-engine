use daraja_core::{
    Timestamp,
    amount::{Amount, TariffSnapshot, Unit},
    crypto::{Clock, Secret},
    identity::{Id, ReceiptId},
    inbound::{
        DepositAttestation, DepositRail, EvidenceSource, InboundQuote, InboundState, QuoteTerms,
        StkAttempt,
    },
    reconcile::*,
    repository::{Repository, StkChange},
    webhook::{Endpoint, Policy, router},
};
use serde_json::{Value, json};
use sqlx::{Connection, Row, SqliteConnection};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
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
struct FakeClock(Arc<AtomicI64>);
impl Clock for FakeClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(self.0.load(Ordering::SeqCst), 0).unwrap()
    }
}
impl FakeClock {
    fn set(&self, n: u64) {
        self.0.store(n as i64, Ordering::SeqCst);
    }
}
#[derive(Clone)]
struct Queries {
    result: Arc<Mutex<QueryState>>,
    stk_calls: Arc<AtomicUsize>,
    c2b_calls: Arc<AtomicUsize>,
    gate: Option<Arc<Semaphore>>,
}
impl Queries {
    fn new(result: QueryState) -> Self {
        Self {
            result: Arc::new(Mutex::new(result)),
            stk_calls: Arc::default(),
            c2b_calls: Arc::default(),
            gate: None,
        }
    }
}
impl InboundQueries for Queries {
    async fn stk(&self, _: &StkAttempt) -> QueryState {
        self.stk_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            let _permit = gate.acquire().await.unwrap();
        }
        *self.result.lock().unwrap()
    }
    async fn c2b(&self, _: &Id, _: &ReceiptId, _: Option<Amount>) -> QueryState {
        self.c2b_calls.fetch_add(1, Ordering::SeqCst);
        *self.result.lock().unwrap()
    }
}
#[derive(Clone)]
struct Records {
    record: Arc<Mutex<Option<DepositAttestation>>>,
    calls: Arc<AtomicUsize>,
}
impl Records {
    fn new(record: Option<DepositAttestation>) -> Self {
        Self {
            record: Arc::new(Mutex::new(record)),
            calls: Arc::default(),
        }
    }
}
impl DepositVerifier for Records {
    async fn evidence(&self, _: &VerificationTarget, _: Timestamp) -> Option<DepositAttestation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.record.lock().unwrap().clone()
    }
}
type Worker = InboundReconciler<Queries, Records, FakeClock>;
struct Fixture {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    repo: Repository,
    clock: FakeClock,
    base: u64,
    address: SocketAddr,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
fn secret(e: Endpoint) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(format!("stage09-synthetic-{e:?}")))
}
impl Fixture {
    async fn new(stk: bool, hold: u64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reconcile.sqlite");
        let repo = Repository::open(&path).await.unwrap();
        let base = chrono::Utc::now().timestamp() as u64;
        let clock = FakeClock(Arc::new(AtomicI64::new((base + 6) as i64)));
        let q = InboundQuote::new(
            id("q"),
            id("TEST0001"),
            id("12345"),
            QuoteTerms {
                expected: kes(10000),
                credit: kes(9900),
                quote_expires_at: Timestamp(base + 3600),
                fx: None,
                tariff: TariffSnapshot {
                    version: id("v1"),
                    fee: kes(100),
                },
                hold_until: Timestamp(base + hold),
            },
        )
        .unwrap();
        repo.insert_quote(&q, Timestamp(base)).await.unwrap();
        if stk {
            repo.change_stk(
                &id("q"),
                0,
                StkChange::Start(StkAttempt::new(id("a"), id("caller"), Timestamp(base)).unwrap()),
                Timestamp(base),
            )
            .await
            .unwrap();
            repo.change_stk(
                &id("q"),
                1,
                StkChange::Acknowledge {
                    attempt: id("a"),
                    merchant: id("merchant"),
                    checkout: id("checkout"),
                },
                Timestamp(base),
            )
            .await
            .unwrap();
        }
        let secrets = Endpoint::ALL
            .into_iter()
            .map(|e| (e, Secret::new(secret(e))))
            .collect::<BTreeMap<_, _>>();
        let policy = Policy::new(id("12345"), secrets).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = router(repo.clone(), policy);
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Self {
            _dir: dir,
            path,
            repo,
            clock,
            base,
            address,
            server,
        }
    }
    fn worker(&self, queries: Queries, records: Records) -> Worker {
        InboundReconciler {
            repository: self.repo.clone(),
            queries,
            verifier: records,
            clock: self.clock.clone(),
            shortcode: id("12345"),
        }
    }
    fn evidence(&self, stk: bool) -> DepositAttestation {
        DepositAttestation {
            source: EvidenceSource::AuthenticatedProviderRecord(id("synthetic-independent-record")),
            receipt: ReceiptId::new("REAL123").unwrap(),
            quote_id: Some(id("q")),
            amount: kes(10000),
            shortcode: id("12345"),
            rail: if stk {
                DepositRail::Stk {
                    checkout_request: id("checkout"),
                }
            } else {
                DepositRail::C2b {
                    account_ref: id("TEST0001"),
                }
            },
            received_at: Timestamp(self.base + 5),
            verified_at: Timestamp(self.base + 6),
        }
    }
    fn date(&self) -> String {
        chrono::DateTime::from_timestamp((self.base + 5) as i64, 0)
            .unwrap()
            .with_timezone(&chrono::FixedOffset::east_opt(10800).unwrap())
            .format("%Y%m%d%H%M%S")
            .to_string()
    }
    fn callback(&self, stk: bool, receipt: &str, amount: u64) -> Value {
        if stk {
            json!({"Body":{"stkCallback":{"MerchantRequestID":"merchant","CheckoutRequestID":"checkout","ResultCode":0,"ResultDesc":"ok","CallbackMetadata":{"Item":[{"Name":"Amount","Value":amount},{"Name":"MpesaReceiptNumber","Value":receipt},{"Name":"TransactionDate","Value":self.date()},{"Name":"PhoneNumber","Value":254712345678_u64}]}}}})
        } else {
            json!({"TransactionType":"Pay Bill","TransID":receipt,"TransTime":self.date(),"TransAmount":amount.to_string(),"BusinessShortCode":"12345","BillRefNumber":"TEST0001","MSISDN":"synthetic-masked"})
        }
    }
    async fn send(&self, e: Endpoint, body: Value) {
        let url = format!(
            "http://{}{}",
            self.address,
            e.path().replace("{secret}", &secret(e))
        );
        let response = reqwest::Client::new()
            .post(url)
            .body(body.to_string())
            .header("content-type", "application/json")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }
    async fn connection(&self) -> SqliteConnection {
        SqliteConnection::connect(self.path.to_str().unwrap())
            .await
            .unwrap()
    }
    async fn financial_counts(&self) -> (i64, i64) {
        let mut c = self.connection().await;
        let receipts = sqlx::query_scalar("SELECT count(*) FROM mpesa_receipts_ledger")
            .fetch_one(&mut c)
            .await
            .unwrap();
        let events = sqlx::query_scalar("SELECT count(*) FROM settlement_events")
            .fetch_one(&mut c)
            .await
            .unwrap();
        (receipts, events)
    }
}

#[tokio::test]
async fn absolute_poll_schedule_survives_restart_and_never_infers_success() {
    let f = Fixture::new(true, 0).await;
    let queries = Queries::new(QueryState::Unresolved);
    let mut w = f.worker(queries.clone(), Records::new(None));
    for elapsed in [34, 35, 64, 65, 109, 110] {
        f.clock.set(f.base + elapsed);
        assert_eq!(
            w.run_once().await.unwrap(),
            [35, 65, 110].contains(&elapsed)
        );
        if elapsed == 35 {
            w.repository = Repository::open(&f.path).await.unwrap();
        }
    }
    assert_eq!(queries.stk_calls.load(Ordering::SeqCst), 3);
    assert_eq!(f.financial_counts().await, (0, 0));
    assert_eq!(
        f.repo.quote(&id("q")).await.unwrap().0.state(),
        InboundState::StkSent
    );
    assert!(!w.run_once().await.unwrap());
}

#[tokio::test]
async fn spike_arrest_retries_are_durable_bounded_and_keep_absolute_deadlines() {
    let f = Fixture::new(true, 0).await;
    let queries = Queries::new(QueryState::Throttled);
    let w = f.worker(queries.clone(), Records::new(None));
    for elapsed in [35, 37, 41, 49, 65, 95] {
        f.clock.set(f.base + elapsed);
        assert!(w.run_once().await.unwrap());
        assert!(
            !w.run_once().await.unwrap(),
            "retry must always be in the future"
        );
    }
    let mut c = f.connection().await;
    let row =
        sqlx::query("SELECT due_at,poll_slot,retry_count FROM durable_jobs WHERE kind='POLL_STK'")
            .fetch_one(&mut c)
            .await
            .unwrap();
    assert_eq!(row.get::<i64, _>("poll_slot"), 1);
    assert_eq!(row.get::<i64, _>("retry_count"), 0);
    assert!(row.get::<i64, _>("due_at") > (f.base + 95) as i64);
    assert_eq!(queries.stk_calls.load(Ordering::SeqCst), 6);
    assert_eq!(f.financial_counts().await, (0, 0));
}

#[tokio::test]
async fn forged_success_failed_or_unknown_query_and_unavailable_fields_remain_unallocated() {
    for state in [QueryState::Unresolved, QueryState::Success] {
        let f = Fixture::new(true, 0).await;
        f.send(Endpoint::Stk, f.callback(true, "REAL123", 100))
            .await;
        let records = Records::new(None);
        let queries = Queries::new(state);
        let w = f.worker(queries, records.clone());
        assert!(w.run_once().await.unwrap());
        assert_eq!(
            records.calls.load(Ordering::SeqCst),
            usize::from(state == QueryState::Success)
        );
        assert_eq!(f.financial_counts().await, (0, 0));
        let mut c = f.connection().await;
        let reason: String = sqlx::query_scalar(
            "SELECT last_reason FROM durable_jobs WHERE kind='INBOUND_CALLBACK'",
        )
        .fetch_one(&mut c)
        .await
        .unwrap();
        assert_eq!(
            reason,
            if state == QueryState::Success {
                "INDEPENDENT_FIELDS_UNAVAILABLE"
            } else {
                "QUERY_UNRESOLVED"
            }
        );
    }
}

#[tokio::test]
async fn wrong_receipt_amount_shortcode_time_and_rail_cannot_poison_legitimate_receipt() {
    for mismatch in 0..6 {
        let f = Fixture::new(true, 0).await;
        f.send(Endpoint::Stk, f.callback(true, "REAL123", 100))
            .await;
        let good = f.evidence(true);
        let mut wrong = good.clone();
        match mismatch {
            0 => wrong.receipt = ReceiptId::new("WRONG").unwrap(),
            1 => wrong.amount = kes(9900),
            2 => wrong.shortcode = id("99999"),
            3 => wrong.received_at = Timestamp(f.base + 4),
            4 => {
                wrong.rail = DepositRail::C2b {
                    account_ref: id("TEST0001"),
                }
            }
            _ => wrong.quote_id = Some(id("other")),
        }
        let records = Records::new(Some(wrong));
        let w = f.worker(Queries::new(QueryState::Success), records.clone());
        w.run_once().await.unwrap();
        assert_eq!(f.financial_counts().await, (0, 0));
        *records.record.lock().unwrap() = Some(good);
        f.clock.set(f.base + 306);
        // The due STK job can recover independently of the rejected callback claim.
        w.run_once().await.unwrap();
        assert_eq!(f.financial_counts().await, (1, 1));
    }
}

#[tokio::test]
async fn missing_callback_recovers_and_events_replay_after_crash_and_disconnect() {
    let f = Fixture::new(true, 0).await;
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "settlement_commit_child"])
        .env("DARAJA_STAGE09_CHILD_DB", &f.path)
        .env("DARAJA_STAGE09_CHILD_BASE", f.base.to_string())
        .output()
        .unwrap();
    assert_eq!(
        child.status.code(),
        Some(41),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    assert_eq!(f.financial_counts().await, (1, 1));
    let reopened = Repository::open(&f.path).await.unwrap();
    let mut stream = IncomingEvents::resume(reopened.clone(), 0).unwrap();
    let first = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.1.quote_id, id("q"));
    drop(stream);
    stream = IncomingEvents::resume(reopened.clone(), 0).unwrap();
    assert_eq!(stream.next().await.unwrap(), first);
    assert!(stream.acknowledge(first.0 + 1).is_err());
    stream.acknowledge(first.0).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), stream.next())
            .await
            .is_err()
    );
    assert!(
        reopened
            .events_after(first.0, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        reopened.quote(&id("q")).await.unwrap().0.state(),
        InboundState::Paid
    );
}

#[tokio::test]
async fn duplicate_and_cross_rail_callbacks_credit_once_without_stk_query_for_c2b() {
    let f = Fixture::new(true, 0).await;
    let body = f.callback(false, "REAL123", 100);
    f.send(Endpoint::C2bConfirmation, body.clone()).await;
    f.send(Endpoint::C2bConfirmation, body).await;
    let queries = Queries::new(QueryState::Success);
    let records = Records::new(Some(f.evidence(false)));
    let w = f.worker(queries.clone(), records.clone());
    w.run_once().await.unwrap();
    assert_eq!(queries.stk_calls.load(Ordering::SeqCst), 0);
    assert_eq!(queries.c2b_calls.load(Ordering::SeqCst), 1);
    f.send(Endpoint::Stk, f.callback(true, "REAL123", 100))
        .await;
    *records.record.lock().unwrap() = Some(f.evidence(true));
    w.run_once().await.unwrap();
    assert_eq!(f.financial_counts().await, (1, 1));
    assert_eq!(
        f.repo.events_after(0, 100).await.unwrap()[0].1.credit,
        kes(9900)
    );
}

#[tokio::test]
async fn competing_workers_and_expired_lease_cannot_double_settle() {
    let f = Fixture::new(true, 0).await;
    let gate = Arc::new(Semaphore::new(0));
    let mut queries = Queries::new(QueryState::Success);
    queries.gate = Some(gate.clone());
    let slow = Arc::new(f.worker(queries.clone(), Records::new(Some(f.evidence(true)))));
    f.clock.set(f.base + 35);
    let worker = slow.clone();
    let pending = tokio::spawn(async move { worker.run_once().await });
    while queries.stk_calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let mut other = f.worker(
        Queries::new(QueryState::Success),
        Records::new(Some(f.evidence(true))),
    );
    other.repository = Repository::open(&f.path).await.unwrap();
    assert!(!other.run_once().await.unwrap());
    f.clock.set(f.base + 65);
    assert!(other.run_once().await.unwrap());
    gate.add_permits(1);
    assert!(matches!(
        pending.await.unwrap(),
        Err(daraja_core::repository::RepositoryError::LeaseLost)
    ));
    assert_eq!(f.financial_counts().await, (1, 1));
}

#[tokio::test]
async fn process_restart_after_claim_resumes_due_work_without_financial_mutation() {
    let f = Fixture::new(true, 0).await;
    f.clock.set(f.base + 35);
    let claim = f
        .repo
        .claim_job(Timestamp(f.base + 35), 30)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claim.kind, "POLL_STK");
    // Abandon the claim, just as a process dying before provider verification would.
    let mut w = f.worker(
        Queries::new(QueryState::Success),
        Records::new(Some(f.evidence(true))),
    );
    w.repository = Repository::open(&f.path).await.unwrap();
    assert!(!w.run_once().await.unwrap());
    f.clock.set(f.base + 65);
    assert!(w.run_once().await.unwrap());
    assert_eq!(f.financial_counts().await, (1, 1));
}

#[tokio::test]
async fn verified_hold_and_amount_or_expiry_exceptions_do_not_issue_early() {
    for case in 0..4 {
        let f = Fixture::new(true, if case == 0 { 90 } else { 0 }).await;
        let mut evidence = f.evidence(true);
        match case {
            1 => evidence.amount = kes(9900),
            2 => evidence.amount = kes(10100),
            3 => {
                evidence.received_at = Timestamp(f.base + 3600);
                evidence.verified_at = Timestamp(f.base + 3601);
            }
            _ => {}
        }
        let w = f.worker(
            Queries::new(QueryState::Success),
            Records::new(Some(evidence)),
        );
        f.clock.set(f.base + if case == 3 { 3601 } else { 35 });
        w.run_once().await.unwrap();
        assert_eq!(f.financial_counts().await, (1, 0));
        assert_eq!(
            f.repo.quote(&id("q")).await.unwrap().0.state(),
            match case {
                0 => InboundState::SettlementHeld,
                1 => InboundState::UnderpaidHeld,
                2 => InboundState::OverpaidHeld,
                _ => InboundState::StkSent,
            }
        );
        if case == 0 {
            *w.verifier.record.lock().unwrap() = None;
            *w.queries.result.lock().unwrap() = QueryState::Unresolved;
            f.clock.set(f.base + 90);
            w.run_once().await.unwrap();
            assert_eq!(f.financial_counts().await, (1, 1));
        }
    }
}

#[tokio::test]
async fn settlement_and_job_completion_roll_back_together_before_restart_retry() {
    let f = Fixture::new(true, 0).await;
    let w = f.worker(
        Queries::new(QueryState::Success),
        Records::new(Some(f.evidence(true))),
    );
    let mut c = f.connection().await;
    sqlx::query("CREATE TRIGGER fail_event BEFORE INSERT ON settlement_events BEGIN SELECT RAISE(ABORT,'synthetic crash boundary'); END").execute(&mut c).await.unwrap();
    f.clock.set(f.base + 35);
    assert!(w.run_once().await.is_err());
    assert_eq!(f.financial_counts().await, (0, 0));
    assert_eq!(
        f.repo.quote(&id("q")).await.unwrap().0.state(),
        InboundState::StkSent
    );
    let completed: Option<i64> =
        sqlx::query_scalar("SELECT completed_at FROM durable_jobs WHERE kind='POLL_STK'")
            .fetch_one(&mut c)
            .await
            .unwrap();
    assert_eq!(completed, None);
    sqlx::query("DROP TRIGGER fail_event")
        .execute(&mut c)
        .await
        .unwrap();
    f.clock.set(f.base + 65);
    let mut restarted = f.worker(
        Queries::new(QueryState::Success),
        Records::new(Some(f.evidence(true))),
    );
    restarted.repository = Repository::open(&f.path).await.unwrap();
    restarted.run_once().await.unwrap();
    assert_eq!(f.financial_counts().await, (1, 1));
}

struct HttpProvider {
    db: std::path::PathBuf,
    callback_address: SocketAddr,
    status_calls: AtomicUsize,
    stk_calls: AtomicUsize,
    stk_mode: Mutex<u8>,
    result: Mutex<Value>,
}
async fn token() -> axum::Json<Value> {
    axum::Json(json!({"access_token":"synthetic", "expires_in":3600}))
}
async fn http_stk(
    axum::extract::State(p): axum::extract::State<Arc<HttpProvider>>,
) -> (axum::http::StatusCode, axum::Json<Value>) {
    p.stk_calls.fetch_add(1, Ordering::SeqCst);
    let mode = *p.stk_mode.lock().unwrap();
    match mode {
        1 => (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            axum::Json(json!({"errorCode":"500.001.1001"})),
        ),
        2 => {
            tokio::time::sleep(Duration::from_millis(200)).await;
            (axum::http::StatusCode::OK, axum::Json(json!({})))
        }
        _ => (
            axum::http::StatusCode::OK,
            axum::Json(
                json!({"ResponseCode":"0","MerchantRequestID":if mode==3 {"wrong"} else {"merchant"},"CheckoutRequestID":"checkout","ResultCode":if mode==4 {"unknown-status"} else {"0"}}),
            ),
        ),
    }
}
async fn http_status(
    axum::extract::State(p): axum::extract::State<Arc<HttpProvider>>,
    axum::Json(body): axum::Json<Value>,
) -> axum::Json<Value> {
    p.status_calls.fetch_add(1, Ordering::SeqCst);
    let mut db = SqliteConnection::connect(p.db.to_str().unwrap())
        .await
        .unwrap();
    let prepared: i64 =
        sqlx::query_scalar("SELECT count(*) FROM inbound_status_queries WHERE receipt=?")
            .bind(body["TransactionID"].as_str().unwrap())
            .fetch_one(&mut db)
            .await
            .unwrap();
    assert!(prepared > 0, "query identity must commit before I/O");
    assert_eq!(body["CommandID"], "TransactionStatusQuery");
    let result = p.result.lock().unwrap().clone();
    let path = Endpoint::StatusResult
        .path()
        .replace("{secret}", &secret(Endpoint::StatusResult));
    let response = reqwest::Client::new()
        .post(format!("http://{}{path}", p.callback_address))
        .header("content-type", "application/json")
        .body(result.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    axum::Json(
        json!({"ResponseCode":"0","OriginatorConversationID":"status-originator","ConversationID":"status-conversation"}),
    )
}
type HttpQueries = ProviderQueries<
    daraja_core::transport::ReqwestTransport,
    FakeClock,
    daraja_core::transport::ReqwestTransport,
    FakeClock,
>;
async fn http_queries(
    f: &Fixture,
) -> (HttpQueries, Arc<HttpProvider>, tokio::task::JoinHandle<()>) {
    use axum::{
        Router,
        routing::{get, post},
    };
    use daraja_core::{
        auth::{CollectionApp, OAuthConfig},
        collection::{CollectionClient, StatusCredentials, StkConfig, TransactionStatusClient},
        transport::ReqwestTransport,
    };
    let provider = Arc::new(HttpProvider {
        db: f.path.clone(),
        callback_address: f.address,
        status_calls: AtomicUsize::new(0),
        stk_calls: AtomicUsize::new(0),
        stk_mode: Mutex::new(0),
        result: Mutex::new(
            json!({"Result":{"ResultCode":0,"OriginatorConversationID":"status-originator","ConversationID":"status-conversation","TransactionID":"REAL123","ResultParameters":{"ResultParameter":[{"Key":"ReceiptNo","Value":"REAL123"},{"Key":"Amount","Value":"100.00"}]}}}),
        ),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = reqwest::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let app = Router::new()
        .route("/oauth", get(token))
        .route("/mpesa/stkpushquery/v1/query", post(http_stk))
        .route("/mpesa/transactionstatus/v1/query", post(http_status))
        .with_state(provider.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let actor = CollectionApp::new(OAuthConfig {
        endpoint: base.join("oauth").unwrap(),
        consumer_key: Secret::new("synthetic"),
        consumer_secret: Secret::new("synthetic"),
    })
    .unwrap();
    let transport =
        || ReqwestTransport::new(Duration::from_millis(30), Duration::from_millis(100)).unwrap();
    let collection = CollectionClient::new(
        transport(),
        base.clone(),
        actor.clone(),
        f.clock.clone(),
        StkConfig {
            shortcode: id("12345"),
            passkey: Secret::new("synthetic"),
        },
    )
    .unwrap();
    let status = TransactionStatusClient::new(
        transport(),
        base,
        StatusCredentials::Collection(actor),
        f.clock.clone(),
    )
    .unwrap();
    (
        ProviderQueries {
            collection,
            status,
            repository: f.repo.clone(),
            clock: f.clock.clone(),
            status_config: InboundStatusConfig {
                initiator: Secret::new("synthetic"),
                security_credential: Secret::new("synthetic"),
                shortcode: id("12345"),
                result_url: Secret::new("https://example.invalid/status/result"),
                timeout_url: Secret::new("https://example.invalid/status/timeout"),
            },
        },
        provider,
        server,
    )
}

#[tokio::test]
async fn real_stk_query_timeout_spike_unknown_and_wrong_merchant_stay_unresolved() {
    let f = Fixture::new(true, 0).await;
    let (queries, p, server) = http_queries(&f).await;
    let a = f.repo.attempt(&id("a")).await.unwrap().1;
    for (mode, expected) in [
        (0, QueryState::Success),
        (1, QueryState::Throttled),
        (2, QueryState::Unresolved),
        (3, QueryState::Unresolved),
        (4, QueryState::Unresolved),
    ] {
        *p.stk_mode.lock().unwrap() = mode;
        assert_eq!(queries.stk(&a).await, expected);
    }
    *p.stk_mode.lock().unwrap() = 2;
    f.send(Endpoint::Stk, f.callback(true, "REAL123", 100))
        .await;
    let records = Records::new(Some(f.evidence(true)));
    let worker = InboundReconciler {
        repository: f.repo.clone(),
        queries,
        verifier: records.clone(),
        clock: f.clock.clone(),
        shortcode: id("12345"),
    };
    worker.run_once().await.unwrap();
    assert_eq!(records.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.financial_counts().await, (0, 0));
    server.abort();
}

#[tokio::test]
async fn c2b_uses_durable_transaction_status_with_early_result_and_reopen_no_resubmission() {
    let f = Fixture::new(false, 0).await;
    f.send(Endpoint::C2bConfirmation, f.callback(false, "REAL123", 100))
        .await;
    let (queries, p, server) = http_queries(&f).await;
    let worker = InboundReconciler {
        repository: f.repo.clone(),
        queries,
        verifier: UnavailableVerifier,
        clock: f.clock.clone(),
        shortcode: id("12345"),
    };
    worker.run_once().await.unwrap();
    assert_eq!(p.status_calls.load(Ordering::SeqCst), 1);
    assert_eq!(p.stk_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.financial_counts().await,
        (0, 0),
        "unsigned asynchronous success is not settlement evidence"
    );
    let mut c = f.connection().await;
    let query_id: String = sqlx::query_scalar("SELECT id FROM inbound_status_queries")
        .fetch_one(&mut c)
        .await
        .unwrap();
    let (mut resumed, p2, server2) = http_queries(&f).await;
    resumed.repository = Repository::open(&f.path).await.unwrap();
    assert_eq!(
        resumed
            .c2b(
                &id(&query_id),
                &ReceiptId::new("REAL123").unwrap(),
                Some(kes(10000))
            )
            .await,
        QueryState::Success
    );
    assert_eq!(p2.status_calls.load(Ordering::SeqCst), 0);
    // Independently reviewed evidence may now authorize the four-gate commit.
    let worker = InboundReconciler {
        repository: f.repo.clone(),
        queries: resumed,
        verifier: Records::new(Some(f.evidence(false))),
        clock: f.clock.clone(),
        shortcode: id("12345"),
    };
    f.clock.set(f.base + 306);
    worker.run_once().await.unwrap();
    assert_eq!(f.financial_counts().await, (1, 1));
    server.abort();
    server2.abort();
}

#[tokio::test]
async fn c2b_status_wrong_receipt_amount_correlation_and_missing_result_remain_pending() {
    for case in 0..5 {
        let f = Fixture::new(false, 0).await;
        let (queries, p, server) = http_queries(&f).await;
        {
            let mut result = p.result.lock().unwrap();
            match case {
                0 => {
                    result["Result"]["TransactionID"] = json!("WRONG123");
                    result["Result"]["ResultParameters"]["ResultParameter"][0]["Value"] =
                        json!("WRONG123");
                }
                1 => {
                    result["Result"]["ResultParameters"]["ResultParameter"][1]["Value"] =
                        json!("99.00")
                }
                2 => result["Result"]["ConversationID"] = json!("unrelated"),
                3 => result["Result"]["ResultCode"] = json!("unknown-code"),
                _ => result["Result"]["OriginatorConversationID"] = json!("timeout-only"),
            }
        }
        assert_eq!(
            queries
                .c2b(
                    &id("status-test"),
                    &ReceiptId::new("REAL123").unwrap(),
                    Some(kes(10000))
                )
                .await,
            QueryState::Unresolved
        );
        // A timeout carries no finality and cannot overwrite a later matching result.
        f.send(Endpoint::StatusTimeout,json!({"Result":{"ResultCode":0,"OriginatorConversationID":"status-originator","ConversationID":"status-conversation"}})).await;
        assert_eq!(
            queries
                .c2b(
                    &id("status-test"),
                    &ReceiptId::new("REAL123").unwrap(),
                    Some(kes(10000))
                )
                .await,
            QueryState::Unresolved
        );
        assert_eq!(p.status_calls.load(Ordering::SeqCst), 1);
        assert_eq!(f.financial_counts().await, (0, 0));
        server.abort();
    }
}

#[tokio::test]
async fn settlement_commit_child() {
    let Ok(path) = std::env::var("DARAJA_STAGE09_CHILD_DB") else {
        return;
    };
    let base: u64 = std::env::var("DARAJA_STAGE09_CHILD_BASE")
        .unwrap()
        .parse()
        .unwrap();
    let repository = Repository::open(path).await.unwrap();
    let evidence = DepositAttestation {
        source: EvidenceSource::AuthenticatedProviderRecord(id("synthetic-independent-record")),
        receipt: ReceiptId::new("REAL123").unwrap(),
        quote_id: Some(id("q")),
        amount: kes(10000),
        shortcode: id("12345"),
        rail: DepositRail::Stk {
            checkout_request: id("checkout"),
        },
        received_at: Timestamp(base + 5),
        verified_at: Timestamp(base + 6),
    };
    let worker = InboundReconciler {
        repository,
        queries: Queries::new(QueryState::Success),
        verifier: Records::new(Some(evidence)),
        clock: FakeClock(Arc::new(AtomicI64::new((base + 35) as i64))),
        shortcode: id("12345"),
    };
    assert!(worker.run_once().await.unwrap());
    std::process::exit(41);
}

#[tokio::test]
async fn migration_preserves_existing_leases_and_schedules_admitted_callbacks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("upgrade.sqlite");
    let migrations = dir.path().join("migrations");
    std::fs::create_dir(&migrations).unwrap();
    for (name, sql) in [
        (
            "0001_accounting.sql",
            include_str!("../migrations/0001_accounting.sql"),
        ),
        (
            "0002_recovery.sql",
            include_str!("../migrations/0002_recovery.sql"),
        ),
        (
            "0003_callback_inbox.sql",
            include_str!("../migrations/0003_callback_inbox.sql"),
        ),
    ] {
        std::fs::write(migrations.join(name), sql).unwrap();
    }
    let mut c = SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::migrate::Migrator::new(migrations.as_path())
        .await
        .unwrap()
        .run(&mut c)
        .await
        .unwrap();
    sqlx::query("INSERT INTO durable_jobs(id,kind,subject,due_at,lease_until,generation) VALUES('stk:old','POLL_STK','old',35,65,7)").execute(&mut c).await.unwrap();
    sqlx::query("INSERT INTO callback_inbox(id,endpoint,received_at,body) VALUES('admitted','/callbacks/c2b/confirmation/{secret}',10,'{}'),('payout','/callbacks/b2c/{secret}',11,'{}')").execute(&mut c).await.unwrap();
    sqlx::query(
        "INSERT INTO callback_jobs(observation_id,due_at) VALUES('admitted',10),('payout',11)",
    )
    .execute(&mut c)
    .await
    .unwrap();
    let repo = Repository::open(&path).await.unwrap();
    let row = sqlx::query(
        "SELECT generation,lease_until,poll_slot,retry_count FROM durable_jobs WHERE id='stk:old'",
    )
    .fetch_one(&mut c)
    .await
    .unwrap();
    assert_eq!(row.get::<i64, _>("generation"), 7);
    assert_eq!(row.get::<i64, _>("lease_until"), 65);
    assert_eq!(row.get::<i64, _>("poll_slot"), 0);
    let claim = repo.claim_job(Timestamp(20), 30).await.unwrap().unwrap();
    assert_eq!(claim.kind, "INBOUND_CALLBACK");
    assert_eq!(claim.subject, id("admitted"));
    assert!(repo.claim_job(Timestamp(20), 30).await.unwrap().is_none());
    assert_eq!(
        repo.claim_job(Timestamp(65), 30)
            .await
            .unwrap()
            .unwrap()
            .generation,
        2
    );
    // The expired older callback lease sorts before the existing STK job.
    let stk = repo.claim_job(Timestamp(65), 30).await.unwrap().unwrap();
    assert_eq!(stk.kind, "POLL_STK");
    assert_eq!(stk.generation, 8);
}

#[tokio::test]
async fn simultaneous_callback_and_poll_workers_commit_one_settlement() {
    let f = Fixture::new(true, 0).await;
    f.send(Endpoint::Stk, f.callback(true, "REAL123", 100))
        .await;
    f.clock.set(f.base + 35);
    let gate = Arc::new(Semaphore::new(0));
    let mut queries = Queries::new(QueryState::Success);
    queries.gate = Some(gate.clone());
    let one = f.worker(queries.clone(), Records::new(Some(f.evidence(true))));
    let mut two = f.worker(queries.clone(), Records::new(Some(f.evidence(true))));
    two.repository = Repository::open(&f.path).await.unwrap();
    let first = tokio::spawn(async move { one.run_once().await });
    let second = tokio::spawn(async move { two.run_once().await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while queries.stk_calls.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    gate.add_permits(2);
    assert!(first.await.unwrap().unwrap());
    assert!(second.await.unwrap().unwrap());
    assert_eq!(f.financial_counts().await, (1, 1));
    let mut c = f.connection().await;
    let completed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM durable_jobs WHERE completed_at IS NOT NULL")
            .fetch_one(&mut c)
            .await
            .unwrap();
    assert_eq!(completed, 2);
}

use super::*;
use daraja_core::{
    identity::ReceiptId,
    inbound::EvidenceSource,
    outbound::{
        FinalOutcome, OutboundProviderQueries, OutboundReconciler, PayoutAttestation, PayoutTarget,
        PayoutVerifier, ProbeEvidence, ReconcileConfig, UnavailablePayoutVerifier,
    },
    reconcile::InboundStatusConfig,
};
use std::sync::atomic::AtomicU64;

#[derive(Clone)]
struct TestClock(Arc<AtomicU64>);
impl TestClock {
    fn at(offset: u64) -> Self {
        Self(Arc::new(AtomicU64::new(time().0 + offset)))
    }
    fn set(&self, offset: u64) {
        self.0.store(time().0 + offset, Ordering::SeqCst);
    }
}
impl Clock for TestClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(self.0.load(Ordering::SeqCst) as i64, 0).unwrap()
    }
}
struct CheckedProbe {
    inner: ReqwestTransport,
    repo: Repository,
    path: std::path::PathBuf,
    crash: Option<String>,
}
impl HttpTransport for CheckedProbe {
    async fn post(&self, request: HttpRequest) -> Result<daraja_core::transport::HttpResponse> {
        let body: Value = serde_json::from_str(request.body.expose()).unwrap();
        let operation = id(body["OriginatorConversationID"].as_str().unwrap());
        let p = self.repo.payout(&operation).await.unwrap().0;
        assert_eq!(p.dispatch().wire_body(), request.body.expose().as_bytes());
        assert_eq!(
            count(&mut db(&self.path).await, "payout_reconcile_attempts").await,
            1
        );
        if self.crash.as_deref() == Some("before") {
            std::process::exit(74);
        }
        let result = self.inner.post(request).await;
        if self.crash.as_deref() == Some("after") {
            std::process::exit(74);
        }
        result
    }
}
type Queries = OutboundProviderQueries<CheckedProbe, ReqwestTransport, TestClock>;
fn queries(
    repo: &Repository,
    path: &std::path::Path,
    server: &Server,
    clock: &TestClock,
) -> Queries {
    queries_base(repo, path, &server.base, clock)
}
fn queries_base(
    repo: &Repository,
    path: &std::path::Path,
    base: &reqwest::Url,
    clock: &TestClock,
) -> Queries {
    let oauth = OAuthConfig {
        endpoint: base.join("disbursement").unwrap(),
        consumer_key: Secret::new("disbursement"),
        consumer_secret: Secret::new("secret"),
    };
    OutboundProviderQueries {
        payout: payout_client(
            CheckedProbe {
                inner: transport(),
                repo: repo.clone(),
                path: path.into(),
                crash: None,
            },
            base.clone(),
        ),
        status: TransactionStatusClient::new(
            transport(),
            base.clone(),
            StatusCredentials::Disbursement(DisbursementApp::new(oauth).unwrap()),
            clock.clone(),
        )
        .unwrap(),
        repository: repo.clone(),
        clock: clock.clone(),
        status_config: InboundStatusConfig {
            initiator: Secret::new("fixture-initiator"),
            security_credential: Secret::new("synthetic-credential"),
            shortcode: id("600000"),
            result_url: Secret::new("https://example.invalid/callbacks/status/result/secret"),
            timeout_url: Secret::new("https://example.invalid/callbacks/status/timeout/secret"),
        },
    }
}
fn worker(
    repo: &Repository,
    path: &std::path::Path,
    server: &Server,
    clock: &TestClock,
) -> OutboundReconciler<Queries, UnavailablePayoutVerifier, TestClock> {
    OutboundReconciler {
        repository: repo.clone(),
        queries: queries(repo, path, server, clock),
        verifier: UnavailablePayoutVerifier,
        clock: clock.clone(),
        config: ReconcileConfig::default(),
    }
}
fn permit(server: &Server, command: &str) -> ProbeEvidence {
    ProbeEvidence {
        endpoint: server
            .base
            .join(if command == "BusinessPayment" {
                "/mpesa/b2c/v3/paymentrequest"
            } else {
                "/mpesa/b2b/v1/paymentrequest"
            })
            .unwrap(),
        command: command.into(),
        evidence_record: id("synthetic-sandbox-equality-retention-trial"),
        verified_until: Timestamp(time().0 + 10000),
        max_operation_age_seconds: 1000,
    }
}
async fn dispatch(repo: &Repository, server: &Server, dest: &str) -> daraja_core::outbound::Payout {
    quote(repo, "quote", dest).await;
    let mut client = payout_client(boundary(repo), server.base.clone());
    client
        .configure_capabilities(
            Capabilities {
                buy_goods: true,
                pay_bill: true,
                b2b_caller_id: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
    OutboundPayments::new(repo.clone(), client, FixedClock, config())
        .unwrap()
        .make_payment(make("key", "quote"))
        .await
        .unwrap()
}
fn unknown() -> Reply {
    Reply::Json(503, json!({"errorCode":"temporary"}))
}
fn tx_ack() -> Value {
    let mut v = status_ack();
    v["TransactionID"] = json!("ACTUAL123");
    v
}
async fn pending_locked(
    repo: &Repository,
    path: &std::path::Path,
    p: &daraja_core::outbound::Payout,
) {
    assert_pending(&repo.payout(p.dispatch().operation_id()).await.unwrap().0);
    let locked: i64 =
        sqlx::query_scalar("SELECT count(*) FROM balance_reservations WHERE released_at IS NULL")
            .fetch_one(&mut db(path).await)
            .await
            .unwrap();
    assert_eq!(locked, 1);
}

#[tokio::test]
async fn safe_probe_is_byte_identical_duplicate_never_paid_and_restart_never_reprobes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let repo = Repository::open(&path).await.unwrap();
    let server = Server::new(vec![
        unknown(),
        Reply::Json(400, json!({"errorCode":"500.002.1001"})),
    ])
    .await;
    let p = dispatch(&repo, &server, "0712345678").await;
    let clock = TestClock::at(35);
    let mut w = worker(&repo, &path, &server, &clock);
    w.config
        .probe_evidence
        .push(permit(&server, "BusinessPayment"));
    assert!(w.run_once().await.unwrap());
    assert_eq!(server.count(), 2);
    assert_eq!(
        repo.payout(p.dispatch().operation_id())
            .await
            .unwrap()
            .0
            .state(),
        OutboundState::Pending
    );
    pending_locked(&repo, &path, &p).await;
    drop(w);
    drop(repo);
    let repo = Repository::open(&path).await.unwrap();
    clock.set(95);
    let mut w = worker(&repo, &path, &server, &clock);
    w.config
        .probe_evidence
        .push(permit(&server, "BusinessPayment"));
    assert!(w.run_once().await.unwrap());
    assert_eq!(server.count(), 2);
    assert_eq!(
        repo.payout(p.dispatch().operation_id())
            .await
            .unwrap()
            .0
            .state(),
        OutboundState::ManualReconciliationRequired
    );
    assert_eq!(
        repo.payout_alerts().await.unwrap()[0].1,
        "NO_SAFE_QUERY_OR_PROBE"
    );
    pending_locked(&repo, &path, &p).await;
}

#[tokio::test]
async fn gates_are_separate_scoped_expiring_and_default_disabled_with_unqueryable_ack() {
    for case in 0..7 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let repo = Repository::open(&path).await.unwrap();
        let server = Server::new(vec![if case == 6 {
            Reply::Json(200, status_ack())
        } else {
            unknown()
        }])
        .await;
        let p = dispatch(
            &repo,
            &server,
            if case == 1 {
                "TILL:123456"
            } else {
                "0712345678"
            },
        )
        .await;
        let clock = TestClock::at(35);
        let mut w = worker(&repo, &path, &server, &clock);
        if case > 0 && case < 6 {
            let mut gate = permit(&server, "BusinessPayment");
            if case == 2 {
                gate.endpoint = "http://127.0.0.1:9/mpesa/b2c/v3/paymentrequest"
                    .parse()
                    .unwrap();
            }
            if case == 3 {
                gate.verified_until = Timestamp(time().0 + 35);
            }
            if case == 4 {
                gate.max_operation_age_seconds = 35;
            }
            if case == 5 {
                gate.command = "BusinessPayBill".into();
            }
            w.config.probe_evidence.push(gate);
        }
        w.run_once().await.unwrap();
        assert_eq!(server.count(), 1);
        assert_eq!(
            repo.payout(p.dispatch().operation_id())
                .await
                .unwrap()
                .0
                .state(),
            OutboundState::ManualReconciliationRequired
        );
        pending_locked(&repo, &path, &p).await;
    }
}

struct Reviewed {
    outcome: FinalOutcome,
}
impl PayoutVerifier for Reviewed {
    async fn evidence(&self, target: &PayoutTarget, _: Timestamp) -> Option<PayoutAttestation> {
        Some(PayoutAttestation {
            source: EvidenceSource::ReviewedStatement(id("reviewed-independent-provider-record")),
            operation_id: target.payout.dispatch().operation_id().clone(),
            payload_hash: target.payout.dispatch().hash(),
            amount: target.payout.dispatch().payload().amount,
            destination: target.payout.dispatch().payload().destination.clone(),
            outcome: self.outcome.clone(),
        })
    }
}
#[tokio::test]
async fn lost_callback_status_uses_transaction_id_budget_escalates_and_late_review_can_settle() {
    for paid in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let repo = Repository::open(&path).await.unwrap();
        let server = Server::new(vec![
            Reply::Json(200, tx_ack()),
            Reply::Json(200, status_ack()),
        ])
        .await;
        let p = dispatch(&repo, &server, "0712345678").await;
        let clock = TestClock::at(35);
        let mut w = worker(&repo, &path, &server, &clock);
        w.config.max_attempts = 1;
        w.run_once().await.unwrap();
        pending_locked(&repo, &path, &p).await;
        {
            let seen = server.state.seen.lock().unwrap();
            assert_eq!(seen[1].path, "/mpesa/transactionstatus/v1/query");
            assert_eq!(seen[1].body["TransactionID"], "ACTUAL123");
            assert!(seen[1].body.get("OriginalConversationID").is_none());
        }
        clock.set(95);
        w.run_once().await.unwrap();
        assert_eq!(
            repo.payout_alerts().await.unwrap()[0].1,
            "POLLING_EXHAUSTED"
        );
        pending_locked(&repo, &path, &p).await;
        let outcome = if paid {
            FinalOutcome::Paid {
                receipt: ReceiptId::new("ACTUAL123").unwrap(),
            }
        } else {
            FinalOutcome::DefinitivelyNotExecuted {
                evidence_record: id("audited-authorized-nonexecution"),
            }
        };
        clock.set(155);
        let reviewed = OutboundReconciler {
            repository: repo.clone(),
            queries: queries(&repo, &path, &server, &clock),
            verifier: Reviewed { outcome },
            clock: clock.clone(),
            config: ReconcileConfig::default(),
        };
        reviewed.run_once().await.unwrap();
        assert_eq!(
            repo.payout(p.dispatch().operation_id())
                .await
                .unwrap()
                .0
                .state(),
            if paid {
                OutboundState::Paid
            } else {
                OutboundState::Failed
            }
        );
        assert_eq!(server.count(), 2);
        assert_eq!(repo.payout_alerts().await.unwrap().len(), 1);
        let released:i64=sqlx::query_scalar("SELECT count(*) FROM balance_reservations WHERE released_at IS NOT NULL AND evidence IS NOT NULL").fetch_one(&mut db(&path).await).await.unwrap();
        assert_eq!(released, 1);
    }
}

#[tokio::test]
async fn concurrent_workers_lease_recovery_and_hard_ttl_never_release_locks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let repo = Repository::open(&path).await.unwrap();
    let server = Server::new(vec![
        Reply::Json(200, tx_ack()),
        Reply::Json(503, json!({"errorCode":"timeout"})),
    ])
    .await;
    let p = dispatch(&repo, &server, "0712345678").await;
    let clock = TestClock::at(35);
    let other = Repository::open(&path).await.unwrap();
    let w = worker(&repo, &path, &server, &clock);
    let w2 = worker(&other, &path, &server, &clock);
    let (a, b) = tokio::join!(w.run_once(), w2.run_once());
    assert_eq!(usize::from(a.unwrap()) + usize::from(b.unwrap()), 1);
    assert_eq!(server.count(), 2);
    pending_locked(&repo, &path, &p).await;
    clock.set(95);
    let abandoned = repo
        .claim_job(Timestamp(time().0 + 95), 30)
        .await
        .unwrap()
        .unwrap();
    assert!(!w.run_once().await.unwrap());
    clock.set(125);
    let mut w = worker(&repo, &path, &server, &clock);
    w.config.automated_poll_seconds = 100;
    w.config.max_pending_ttl_seconds = 125;
    assert!(w.run_once().await.unwrap());
    assert!(matches!(
        repo.finish_job(&abandoned, Timestamp(time().0 + 125), None)
            .await,
        Err(RepositoryError::LeaseLost)
    ));
    assert_eq!(repo.payout_alerts().await.unwrap()[0].1, "MAX_PENDING_TTL");
    assert_eq!(server.count(), 2);
    pending_locked(&repo, &path, &p).await;
}

#[tokio::test]
async fn ttl_preempts_a_future_job_and_manual_escalation_is_atomic_with_alert() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let repo = Repository::open(&path).await.unwrap();
    let server = Server::new(vec![unknown()]).await;
    let p = dispatch(&repo, &server, "0712345678").await;
    let clock = TestClock::at(10);
    let mut w = worker(&repo, &path, &server, &clock);
    w.config.automated_poll_seconds = 10;
    w.config.max_pending_ttl_seconds = 10;
    sqlx::query("CREATE TRIGGER block_alert BEFORE INSERT ON payout_alerts BEGIN SELECT RAISE(ABORT,'fault'); END").execute(&mut db(&path).await).await.unwrap();
    assert!(w.run_once().await.is_err());
    assert_eq!(
        repo.payout(p.dispatch().operation_id())
            .await
            .unwrap()
            .0
            .state(),
        OutboundState::UnknownInFlight
    );
    assert!(repo.payout_alerts().await.unwrap().is_empty());
    pending_locked(&repo, &path, &p).await;
    sqlx::query("DROP TRIGGER block_alert")
        .execute(&mut db(&path).await)
        .await
        .unwrap();
    clock.set(40);
    w.run_once().await.unwrap();
    assert_eq!(repo.payout_alerts().await.unwrap()[0].1, "MAX_PENDING_TTL");
    pending_locked(&repo, &path, &p).await;
    assert_eq!(server.count(), 1);
}

async fn post_callback(
    base: &reqwest::Url,
    endpoint: Endpoint,
    body: Value,
) -> reqwest::StatusCode {
    reqwest::Client::new()
        .post(
            base.join(
                &endpoint
                    .path()
                    .replace("{secret}", &callback_secret(endpoint)),
            )
            .unwrap(),
        )
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap()
        .status()
}
struct IndependentAfterQuery;
impl PayoutVerifier for IndependentAfterQuery {
    async fn evidence(&self, target: &PayoutTarget, now: Timestamp) -> Option<PayoutAttestation> {
        if target.status_callbacks.len() != 1 {
            return None;
        }
        // Synthetic independent statement; the callback only selects when it becomes available.
        Reviewed {
            outcome: FinalOutcome::Paid {
                receipt: ReceiptId::new("ACTUAL123").unwrap(),
            },
        }
        .evidence(target, now)
        .await
    }
}
#[tokio::test]
async fn enabled_probe_acknowledges_then_queries_and_callbacks_need_independent_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let repo = Repository::open(&path).await.unwrap();
    let server=Server::new(vec![unknown(),Reply::Json(200,tx_ack()),Reply::Json(200,json!({"ResponseCode":"0","OriginatorConversationID":"status-origin","ConversationID":"status-conversation"}))]).await;
    let p = dispatch(&repo, &server, "0712345678").await;
    let clock = TestClock::at(35);
    let mut w = worker(&repo, &path, &server, &clock);
    w.config
        .probe_evidence
        .push(permit(&server, "BusinessPayment"));
    w.run_once().await.unwrap();
    assert_eq!(
        repo.payout(p.dispatch().operation_id())
            .await
            .unwrap()
            .0
            .state(),
        OutboundState::Pending
    );
    clock.set(95);
    w.run_once().await.unwrap();
    assert_eq!(server.count(), 3);
    let (base, task) = ingress(&repo).await;
    let result = json!({"Result":{"ResultCode":0,"ResultDesc":"ok","OriginatorConversationID":"status-origin","ConversationID":"status-conversation","TransactionID":"ACTUAL123"}});
    let mut wrong = result.clone();
    wrong["Result"]["ConversationID"] = json!("unrelated");
    assert_eq!(
        post_callback(&base, Endpoint::StatusResult, wrong).await,
        200
    );
    let reviewed = OutboundReconciler {
        repository: repo.clone(),
        queries: queries(&repo, &path, &server, &clock),
        verifier: IndependentAfterQuery,
        clock: clock.clone(),
        config: ReconcileConfig {
            max_attempts: 2,
            ..Default::default()
        },
    };
    clock.set(155);
    reviewed.run_once().await.unwrap();
    assert_eq!(
        repo.payout(p.dispatch().operation_id())
            .await
            .unwrap()
            .0
            .state(),
        OutboundState::ManualReconciliationRequired
    );
    assert_eq!(
        post_callback(&base, Endpoint::StatusResult, result.clone()).await,
        200
    );
    assert_eq!(
        post_callback(&base, Endpoint::StatusResult, result).await,
        200
    );
    clock.set(215);
    w.run_once().await.unwrap();
    pending_locked(&repo, &path, &p).await;
    assert_eq!(server.count(), 3);
    clock.set(275);
    reviewed.run_once().await.unwrap();
    let resolved = repo.payout(p.dispatch().operation_id()).await.unwrap().0;
    assert_eq!(resolved.state(), OutboundState::Paid);
    assert!(
        matches!(&resolved.final_evidence().unwrap().outcome, FinalOutcome::Paid{receipt} if receipt.as_str()=="ACTUAL123")
    );
    assert_eq!(server.count(), 3);
    task.abort();
}

#[tokio::test]
async fn contradictory_payout_and_status_callbacks_escalate_without_finality() {
    for status in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let repo = Repository::open(&path).await.unwrap();
        let server=Server::new(if status {vec![Reply::Json(200,tx_ack()),Reply::Json(200,json!({"ResponseCode":"0","OriginatorConversationID":"status-origin","ConversationID":"status-conversation"}))]} else {vec![Reply::Json(200,status_ack())]}).await;
        let p = dispatch(&repo, &server, "0712345678").await;
        let clock = TestClock::at(35);
        let w = worker(&repo, &path, &server, &clock);
        if status {
            w.run_once().await.unwrap();
            clock.set(95);
        }
        let (base, task) = ingress(&repo).await;
        let endpoint = if status {
            Endpoint::StatusResult
        } else {
            Endpoint::B2c
        };
        let mut body = json!({"Result":{"ResultCode":0,"ResultDesc":"ok","OriginatorConversationID":if status {"status-origin"} else {"originator-1"},"ConversationID":if status {"status-conversation"} else {"conversation-1"},"TransactionID":"ACTUAL123"}});
        assert_eq!(post_callback(&base, endpoint, body.clone()).await, 200);
        body["Result"]["ResultCode"] = json!(1);
        assert_eq!(post_callback(&base, endpoint, body).await, 200);
        w.run_once().await.unwrap();
        assert_eq!(
            repo.payout_alerts().await.unwrap()[0].1,
            "CONTRADICTORY_OBSERVATIONS"
        );
        pending_locked(&repo, &path, &p).await;
        assert_eq!(server.count(), if status { 2 } else { 1 });
        task.abort();
    }
}

#[tokio::test]
async fn probe_and_query_transport_failures_and_cancellation_preserve_pending() {
    for reply in [
        Reply::HeadersHang,
        Reply::BodyHang,
        Reply::Raw(200, "broken"),
        Reply::Json(500, json!({"errorCode":"no"})),
        Reply::Json(503, json!({"errorCode":"no"})),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let repo = Repository::open(&path).await.unwrap();
        let server = Server::new(vec![unknown(), reply]).await;
        let p = dispatch(&repo, &server, "0712345678").await;
        let clock = TestClock::at(35);
        let mut w = worker(&repo, &path, &server, &clock);
        w.config
            .probe_evidence
            .push(permit(&server, "BusinessPayment"));
        w.run_once().await.unwrap();
        pending_locked(&repo, &path, &p).await;
        clock.set(95);
        w.run_once().await.unwrap();
        assert_eq!(server.count(), 2);
        assert_eq!(
            repo.payout(p.dispatch().operation_id())
                .await
                .unwrap()
                .0
                .state(),
            OutboundState::ManualReconciliationRequired
        );
        pending_locked(&repo, &path, &p).await;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let repo = Repository::open(&path).await.unwrap();
    let server = Server::new(vec![Reply::Json(200, tx_ack()), Reply::HeadersHang]).await;
    let p = dispatch(&repo, &server, "0712345678").await;
    let clock = TestClock::at(35);
    let w = worker(&repo, &path, &server, &clock);
    let task = tokio::spawn(async move { w.run_once().await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while server.count() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    pending_locked(&repo, &path, &p).await;
    clock.set(65);
    let mut w = worker(&repo, &path, &server, &clock);
    w.config.max_attempts = 1;
    w.run_once().await.unwrap();
    assert_eq!(server.count(), 2);
    assert_eq!(
        repo.payout_alerts().await.unwrap()[0].1,
        "POLLING_EXHAUSTED"
    );
    pending_locked(&repo, &path, &p).await;
}

#[tokio::test]
async fn reconciliation_crash_child() {
    let Ok(path) = std::env::var("DARAJA_STAGE13_DB") else {
        return;
    };
    let base: reqwest::Url = std::env::var("DARAJA_STAGE13_BASE")
        .unwrap()
        .parse()
        .unwrap();
    let repo = Repository::open(&path).await.unwrap();
    let clock = TestClock::at(35);
    let mut q = queries_base(&repo, std::path::Path::new(&path), &base, &clock);
    q.payout = payout_client(
        CheckedProbe {
            inner: transport(),
            repo: repo.clone(),
            path: path.into(),
            crash: Some(std::env::var("DARAJA_STAGE13_CRASH").unwrap()),
        },
        base.clone(),
    );
    let mut config = ReconcileConfig::default();
    config.probe_evidence.push(ProbeEvidence {
        endpoint: base.join("/mpesa/b2c/v3/paymentrequest").unwrap(),
        command: "BusinessPayment".into(),
        evidence_record: id("synthetic-trial"),
        verified_until: Timestamp(time().0 + 10000),
        max_operation_age_seconds: 1000,
    });
    let w = OutboundReconciler {
        repository: repo,
        queries: q,
        verifier: UnavailablePayoutVerifier,
        clock,
        config,
    };
    let _ = w.run_once().await;
    panic!("crash point not reached");
}
#[tokio::test]
async fn abrupt_exits_before_and_after_probe_cannot_issue_a_second_probe() {
    for (phase, extra) in [("before", 0), ("after", 1)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let repo = Repository::open(&path).await.unwrap();
        let server = Server::new(vec![unknown(), Reply::Json(200, tx_ack())]).await;
        let p = dispatch(&repo, &server, "0712345678").await;
        let child_path = path.clone();
        let base = server.base.to_string();
        let status = tokio::task::spawn_blocking(move || {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "outbound_dispatch::reconcile_stage13::reconciliation_crash_child",
                    "--nocapture",
                ])
                .env("DARAJA_STAGE13_DB", child_path)
                .env("DARAJA_STAGE13_BASE", base)
                .env("DARAJA_STAGE13_CRASH", phase)
                .status()
                .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(status.code(), Some(74));
        assert_eq!(server.count(), 1 + extra);
        pending_locked(&repo, &path, &p).await;
        drop(repo);
        let repo = Repository::open(&path).await.unwrap();
        let clock = TestClock::at(65);
        let mut w = worker(&repo, &path, &server, &clock);
        w.config
            .probe_evidence
            .push(permit(&server, "BusinessPayment"));
        w.run_once().await.unwrap();
        assert_eq!(server.count(), 1 + extra);
        assert_eq!(
            repo.payout_alerts().await.unwrap()[0].1,
            "NO_SAFE_QUERY_OR_PROBE"
        );
        pending_locked(&repo, &path, &p).await;
    }
}

#[derive(Clone)]
struct OAuthDelayClock(Arc<Provider>);
impl Clock for OAuthDelayClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        // Simulate wall-clock expiry while a fresh OAuth actor obtains credentials.
        FixedClock.now()
            + chrono::Duration::seconds(if self.0.auth_calls.load(Ordering::SeqCst) > 1 {
                1000
            } else {
                35
            })
    }
}
#[tokio::test]
async fn expiry_during_oauth_prevents_probe_send_and_stale_worker_cannot_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let repo = Repository::open(&path).await.unwrap();
    let server = Server::new(vec![unknown()]).await;
    let p = dispatch(&repo, &server, "0712345678").await;
    let clock = OAuthDelayClock(server.state.clone());
    let q = OutboundProviderQueries {
        payout: payout_client(
            CheckedProbe {
                inner: transport(),
                repo: repo.clone(),
                path: path.clone(),
                crash: None,
            },
            server.base.clone(),
        ),
        status: TransactionStatusClient::new(
            transport(),
            server.base.clone(),
            StatusCredentials::Disbursement(
                DisbursementApp::new(server.config("disbursement")).unwrap(),
            ),
            clock.clone(),
        )
        .unwrap(),
        repository: repo.clone(),
        clock: clock.clone(),
        status_config: InboundStatusConfig {
            initiator: Secret::new("fixture"),
            security_credential: Secret::new("fixture"),
            shortcode: id("600000"),
            result_url: Secret::new("https://example.invalid/result"),
            timeout_url: Secret::new("https://example.invalid/timeout"),
        },
    };
    let mut cfg = ReconcileConfig::default();
    let mut gate = permit(&server, "BusinessPayment");
    gate.verified_until = Timestamp(time().0 + 100);
    cfg.probe_evidence.push(gate);
    let w = OutboundReconciler {
        repository: repo.clone(),
        queries: q,
        verifier: UnavailablePayoutVerifier,
        clock,
        config: cfg,
    };
    assert!(matches!(
        w.run_once().await,
        Err(RepositoryError::LeaseLost)
    ));
    assert_eq!(server.count(), 1);
    assert_eq!(server.state.auth_calls.load(Ordering::SeqCst), 2);
    pending_locked(&repo, &path, &p).await;
    w.run_once().await.unwrap();
    assert_eq!(
        repo.payout_alerts().await.unwrap()[0].1,
        "NO_SAFE_QUERY_OR_PROBE"
    );
    assert_eq!(server.count(), 1);
    pending_locked(&repo, &path, &p).await;
}

struct WrongValue;
impl PayoutVerifier for WrongValue {
    async fn evidence(&self, target: &PayoutTarget, now: Timestamp) -> Option<PayoutAttestation> {
        let mut e = Reviewed {
            outcome: FinalOutcome::Paid {
                receipt: ReceiptId::new("ACTUAL123").unwrap(),
            },
        }
        .evidence(target, now)
        .await
        .unwrap();
        e.amount = kes(1);
        Some(e)
    }
}
#[tokio::test]
async fn mismatched_final_evidence_and_final_write_fault_cannot_release_reservation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let repo = Repository::open(&path).await.unwrap();
    let server = Server::new(vec![unknown()]).await;
    let p = dispatch(&repo, &server, "0712345678").await;
    let clock = TestClock::at(35);
    let w = OutboundReconciler {
        repository: repo.clone(),
        queries: queries(&repo, &path, &server, &clock),
        verifier: WrongValue,
        clock: clock.clone(),
        config: ReconcileConfig::default(),
    };
    assert!(matches!(
        w.run_once().await,
        Err(RepositoryError::Domain(Error::EvidenceMismatch))
    ));
    pending_locked(&repo, &path, &p).await;
    clock.set(65);
    sqlx::query("CREATE TRIGGER block_final BEFORE UPDATE ON outbound_payouts WHEN NEW.state='Paid' BEGIN SELECT RAISE(ABORT,'fault'); END").execute(&mut db(&path).await).await.unwrap();
    let w = OutboundReconciler {
        repository: repo.clone(),
        queries: queries(&repo, &path, &server, &clock),
        verifier: Reviewed {
            outcome: FinalOutcome::Paid {
                receipt: ReceiptId::new("ACTUAL123").unwrap(),
            },
        },
        clock: clock.clone(),
        config: ReconcileConfig::default(),
    };
    assert!(w.run_once().await.is_err());
    pending_locked(&repo, &path, &p).await;
    sqlx::query("DROP TRIGGER block_final")
        .execute(&mut db(&path).await)
        .await
        .unwrap();
    clock.set(95);
    w.run_once().await.unwrap();
    assert_eq!(
        repo.payout(p.dispatch().operation_id())
            .await
            .unwrap()
            .0
            .state(),
        OutboundState::Paid
    );
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn one_receipt_cannot_finalize_two_payouts_through_worker_or_direct_repository() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let repo = Repository::open(&path).await.unwrap();
    let server = Server::new(vec![unknown(), unknown()]).await;
    let first = dispatch(&repo, &server, "0712345678").await;
    quote(&repo, "second-quote", "0798765432").await;
    let second = service(&repo, &server)
        .make_payment(make("second-key", "second-quote"))
        .await
        .unwrap();
    let clock = TestClock::at(35);
    let verifier = Reviewed {
        outcome: FinalOutcome::Paid {
            receipt: ReceiptId::new("ACTUAL123").unwrap(),
        },
    };
    let w = OutboundReconciler {
        repository: repo.clone(),
        queries: queries(&repo, &path, &server, &clock),
        verifier,
        clock,
        config: ReconcileConfig::default(),
    };
    w.run_once().await.unwrap();
    assert!(w.run_once().await.is_err());
    let payouts = [first, second];
    let mut locked = 0;
    for p in &payouts {
        let (saved, version) = repo.payout(p.dispatch().operation_id()).await.unwrap();
        if saved.state() == OutboundState::Paid {
            continue;
        }
        locked += 1;
        assert_pending(&saved);
        let evidence = PayoutAttestation {
            source: EvidenceSource::ReviewedStatement(id("independent-record")),
            operation_id: p.dispatch().operation_id().clone(),
            payload_hash: p.dispatch().hash(),
            amount: p.dispatch().payload().amount,
            destination: p.dispatch().payload().destination.clone(),
            outcome: FinalOutcome::Paid {
                receipt: ReceiptId::new("ACTUAL123").unwrap(),
            },
        };
        assert!(
            repo.change_payout(
                p.dispatch().operation_id(),
                version,
                daraja_core::repository::PayoutChange::Resolve(evidence),
                Timestamp(time().0 + 35)
            )
            .await
            .is_err()
        );
        assert_pending(&repo.payout(p.dispatch().operation_id()).await.unwrap().0);
    }
    assert_eq!(locked, 1);
    let released: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM balance_reservations WHERE released_at IS NOT NULL",
    )
    .fetch_one(&mut db(&path).await)
    .await
    .unwrap();
    assert_eq!(released, 1);
}

use super::*;
use daraja_core::{
    amount::TariffSnapshot,
    liquidity::{
        AccountRole, BalanceEvidence, BalanceMonitor, BalanceTarget, BalanceVerifier,
        LiquidityPolicy, SweepConfig, UnavailableBalanceVerifier,
    },
    payout::{BalanceClient, LinkedAccounts},
};
use std::sync::atomic::AtomicU64;
#[derive(Clone)]
struct LiquidityClock(Arc<AtomicU64>);
impl LiquidityClock {
    fn new() -> Self {
        Self(Arc::new(AtomicU64::new(time().0)))
    }
    fn set(&self, offset: u64) {
        self.0.store(time().0 + offset, Ordering::SeqCst);
    }
}
impl Clock for LiquidityClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(self.0.load(Ordering::SeqCst) as i64, 0).unwrap()
    }
}
async fn setup(repo: &Repository) {
    repo.configure_liquidity(LiquidityPolicy {
        collection: id("600001"),
        disbursement: id("600000"),
        minimum: kes(100),
        max_age_seconds: 180,
        poll_interval_seconds: 60,
    })
    .await
    .unwrap();
}
fn remitter(account: &str) -> Remitter {
    let key = SecurityCredentialKey::from_certificate(
        Environment::Sandbox,
        include_bytes!("../fixtures/rsa-cert.txt"),
    )
    .unwrap();
    Remitter::new(
        Environment::Sandbox,
        &key,
        Secret::new("fixture"),
        &Secret::new("password"),
        id(account),
    )
    .unwrap()
}
fn ack(role: &str) -> Value {
    json!({"ResponseCode":"0","OriginatorConversationID":format!("origin-{role}"),"ConversationID":format!("conv-{role}"),"ResponseDescription":"accepted"})
}
fn observation(role: &str, amount: &str) -> Value {
    json!({"Result":{"ResultCode":0,"OriginatorConversationID":format!("origin-{role}"),"ConversationID":format!("conv-{role}"),"ResultParameters":{"ResultParameter":[{"Key":"SyntheticAvailableKES","Value":amount}]}}})
}
struct BalanceBoundary {
    inner: ReqwestTransport,
    path: std::path::PathBuf,
    callback: Option<reqwest::Url>,
}
impl HttpTransport for BalanceBoundary {
    async fn post(&self, request: HttpRequest) -> Result<daraja_core::transport::HttpResponse> {
        let body: Value = serde_json::from_str(request.body.expose()).unwrap();
        let account = body["PartyA"].as_str().unwrap();
        let mut raw = db(&self.path).await;
        let saved: String = sqlx::query_scalar(
            "SELECT wire_body FROM balance_queries WHERE account=? ORDER BY sequence DESC LIMIT 1",
        )
        .bind(account)
        .fetch_one(&mut raw)
        .await
        .unwrap();
        assert_eq!(saved, request.body.expose());
        if let Some(callback) = &self.callback {
            let role = if account == "600001" {
                "collection"
            } else {
                "disbursement"
            };
            let response = reqwest::Client::new()
                .post(callback.clone())
                .header("content-type", "application/json")
                .body(observation(role, "123.45").to_string())
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
        }
        self.inner.post(request).await
    }
}
fn balance_client(
    server: &Server,
    clock: &LiquidityClock,
    path: &std::path::Path,
    collection: bool,
    callback: Option<reqwest::Url>,
) -> BalanceClient<BalanceBoundary, LiquidityClock> {
    let credentials = if collection {
        StatusCredentials::Collection(CollectionApp::new(server.config("collection")).unwrap())
    } else {
        StatusCredentials::Disbursement(
            DisbursementApp::new(server.config("disbursement")).unwrap(),
        )
    };
    BalanceClient::new(
        BalanceBoundary {
            inner: transport(),
            path: path.into(),
            callback,
        },
        server.base.clone(),
        credentials,
        remitter(if collection { "600001" } else { "600000" }),
        clock.clone(),
    )
    .unwrap()
}
// This fixture supplies independent synthetic evidence, never a live provider guarantee.
struct SyntheticBalanceVerifier;
impl BalanceVerifier for SyntheticBalanceVerifier {
    async fn evidence(&self, target: &BalanceTarget, _: Timestamp) -> Option<BalanceEvidence> {
        let observation = target.callbacks.iter().find_map(|body| {
            let result = StatusResult::parse(body.expose()).ok()?;
            if result.result_code.as_str() != "0" {
                return None;
            }
            Amount::kes_decimal(result.parameters.get("SyntheticAvailableKES")?.expose()).ok()
        })?;
        Some(BalanceEvidence {
            query_id: target.query_id.clone(),
            account: target.account.clone(),
            role: target.role,
            available: observation,
            evidence_record: id("synthetic-independent-balance"),
        })
    }
}
#[tokio::test]
async fn monitor_commits_before_io_correlates_early_replies_and_recovers_without_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let repo = Repository::open(&path).await.unwrap();
    setup(&repo).await;
    let (base, ingress_task) = ingress(&repo).await;
    let callback = base
        .join(&format!(
            "callbacks/balance/result/{}",
            callback_secret(Endpoint::BalanceResult)
        ))
        .unwrap();
    let server = Server::new(vec![
        Reply::Json(200, ack("collection")),
        Reply::Json(200, ack("disbursement")),
        Reply::Json(503, json!({"errorCode":"503"})),
        Reply::HeadersHang,
    ])
    .await;
    let clock = LiquidityClock::new();
    let monitor = BalanceMonitor {
        repository: repo.clone(),
        collection: balance_client(&server, &clock, &path, true, Some(callback.clone())),
        disbursement: balance_client(&server, &clock, &path, false, Some(callback)),
        clock: clock.clone(),
        verifier: UnavailableBalanceVerifier,
        result_url: Secret::new("https://example.invalid/result"),
        timeout_url: Secret::new("https://example.invalid/timeout"),
    };
    monitor.run_once().await.unwrap();
    assert_eq!(server.count(), 2);
    assert!(
        repo.float_status(AccountRole::Disbursement, time())
            .await
            .unwrap()
            .available
            .is_none()
    );
    let targets = repo
        .balance_targets(AccountRole::Disbursement)
        .await
        .unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].callbacks.len(), 1);
    // A wrong conversation has been admitted but cannot bind the query.
    let mut forged = observation("disbursement", "999999.00");
    forged["Result"]["ConversationID"] = json!("wrong");
    let url = base
        .join(&format!(
            "callbacks/balance/result/{}",
            callback_secret(Endpoint::BalanceResult)
        ))
        .unwrap();
    assert!(
        reqwest::Client::new()
            .post(url)
            .header("content-type", "application/json")
            .body(forged.to_string())
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let monitor = BalanceMonitor {
        verifier: SyntheticBalanceVerifier,
        repository: monitor.repository,
        collection: monitor.collection,
        disbursement: monitor.disbursement,
        clock: monitor.clock,
        result_url: monitor.result_url,
        timeout_url: monitor.timeout_url,
    };
    monitor.run_once().await.unwrap();
    assert_eq!(server.count(), 2);
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, time())
            .await
            .unwrap()
            .balance,
        Some(kes(12345))
    );
    clock.set(60);
    monitor.run_once().await.unwrap();
    assert_eq!(server.count(), 4);
    assert_eq!(count(&mut db(&path).await, "balance_queries").await, 4);
    drop(monitor);
    ingress_task.abort();
    repo.close().await;
    let repo = Repository::open(&path).await.unwrap();
    let monitor = BalanceMonitor {
        repository: repo.clone(),
        collection: balance_client(&server, &clock, &path, true, None),
        disbursement: balance_client(&server, &clock, &path, false, None),
        clock: clock.clone(),
        verifier: UnavailableBalanceVerifier,
        result_url: Secret::new("https://example.invalid/result"),
        timeout_url: Secret::new("https://example.invalid/timeout"),
    };
    monitor.run_once().await.unwrap();
    assert_eq!(server.count(), 4);
    assert!(
        repo.float_status(AccountRole::Disbursement, Timestamp(time().0 + 180))
            .await
            .unwrap()
            .available
            .is_none()
    );
}
fn sweep_config() -> SweepConfig {
    SweepConfig {
        source: id("600001"),
        utility: id("600000"),
        amount: kes(1000),
        tariff: TariffSnapshot {
            version: id("synthetic-sweep-fee"),
            fee: kes(100),
        },
        max_daily: kes(2200),
        cooldown_seconds: 120,
    }
}
fn sweep_service(
    repo: &Repository,
    server: &Server,
    clock: &LiquidityClock,
    enabled: bool,
) -> OutboundPayments<ReqwestTransport, LiquidityClock> {
    let mut client = PayoutClient::new(
        transport(),
        server.base.clone(),
        DisbursementApp::new(server.config("disbursement")).unwrap(),
        remitter("600001"),
        100000,
    )
    .unwrap();
    if enabled {
        client
            .configure_capabilities(
                Capabilities {
                    b2b_caller_id: true,
                    linked_account_sweep: true,
                    transfer_mmf_to_utility: true,
                    ..Default::default()
                },
                Some(LinkedAccounts {
                    source: id("600001"),
                    utility: id("600000"),
                }),
            )
            .unwrap();
    }
    OutboundPayments::new(repo.clone(), client, clock.clone(), config()).unwrap()
}
async fn seed_sweep(repo: &Repository) {
    setup(repo).await;
    super::super::liquidity_fixture::observe(
        repo,
        AccountRole::Collection,
        "600001",
        "source",
        time().0,
        10000,
    )
    .await;
    super::super::liquidity_fixture::observe(
        repo,
        AccountRole::Disbursement,
        "600000",
        "utility",
        time().0,
        0,
    )
    .await;
}
#[tokio::test]
async fn sweeps_are_opt_in_linked_limited_and_ambiguous_errors_never_replenish_float() {
    for reply in [
        Reply::Json(503, json!({"errorCode":"503"})),
        Reply::HeadersHang,
        Reply::BodyHang,
        Reply::Json(200, status_ack()),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let repo = Repository::open(&path).await.unwrap();
        seed_sweep(&repo).await;
        let server = Server::new(vec![reply]).await;
        let clock = LiquidityClock::new();
        let cfg = sweep_config();
        let service = sweep_service(&repo, &server, &clock, false);
        assert!(service.sweep_if_needed(None).await.unwrap().is_none());
        assert!(service.sweep_if_needed(Some(&cfg)).await.is_err());
        assert_eq!(server.count(), 0);
        let service = sweep_service(&repo, &server, &clock, true);
        let mut bad = cfg.clone();
        bad.utility = id("600002");
        assert!(service.sweep_if_needed(Some(&bad)).await.is_err());
        assert_eq!(server.count(), 0);
        let p = service.sweep_if_needed(Some(&cfg)).await.unwrap().unwrap();
        assert_eq!(server.count(), 1);
        assert_eq!(p.state().cdk_state(), CdkPaymentState::Pending);
        let source = repo
            .float_status(AccountRole::Collection, time())
            .await
            .unwrap();
        assert_eq!(source.pending_liability, kes(1100));
        assert_eq!(source.available, Some(kes(8900)));
        assert_eq!(
            repo.float_status(AccountRole::Disbursement, time())
                .await
                .unwrap()
                .available,
            Some(kes(0))
        );
        assert_eq!(
            service
                .sweep_if_needed(Some(&cfg))
                .await
                .unwrap()
                .unwrap()
                .dispatch(),
            p.dispatch()
        );
        assert_eq!(server.count(), 1);
        let now = Timestamp(time().0 + 120);
        super::super::liquidity_fixture::observe(
            &repo,
            AccountRole::Collection,
            "600001",
            "new-source",
            now.0,
            10000,
        )
        .await;
        clock.set(120);
        assert!(service.sweep_if_needed(Some(&cfg)).await.is_err());
        assert_eq!(server.count(), 1);
        repo.recover_dispatching_payouts(now).await.unwrap();
        drop(service);
        repo.close().await;
        let repo = Repository::open(&path).await.unwrap();
        let service = sweep_service(&repo, &server, &clock, true);
        assert!(service.sweep_if_needed(Some(&cfg)).await.is_err());
        assert_eq!(server.count(), 1);
        assert_eq!(
            repo.payout(p.dispatch().operation_id())
                .await
                .unwrap()
                .0
                .state()
                .cdk_state(),
            CdkPaymentState::Pending
        );
    }
}

#[derive(Clone)]
struct AdmissionDelayClock(Arc<Provider>);
impl Clock for AdmissionDelayClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(
            (time().0
                + if self.0.auth_calls.load(Ordering::SeqCst) > 0 {
                    180
                } else {
                    0
                }) as i64,
            0,
        )
        .unwrap()
    }
}
#[tokio::test]
async fn float_expiring_during_oauth_stops_send_and_preserves_pending_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let repo = Repository::open(&path).await.unwrap();
    setup(&repo).await;
    super::super::liquidity_fixture::observe(
        &repo,
        AccountRole::Disbursement,
        "600000",
        "initial",
        time().0,
        1_000_000,
    )
    .await;
    quote(&repo, "q", "0712345678").await;
    let server = Server::new(vec![]).await;
    let clock = AdmissionDelayClock(server.state.clone());
    let service = OutboundPayments::new(
        repo.clone(),
        payout_client(transport(), server.base.clone()),
        clock,
        config(),
    )
    .unwrap();
    assert!(matches!(
        service.make_payment(make("key", "q")).await,
        Err(daraja_core::outbound::DispatchError::Repository(
            RepositoryError::Domain(Error::LiquidityUnavailable)
        ))
    ));
    assert_eq!(server.count(), 0);
    let p = service.make_payment(make("key", "q")).await.unwrap();
    assert_eq!(p.state().cdk_state(), CdkPaymentState::Pending);
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(time().0 + 180))
            .await
            .unwrap()
            .pending_liability,
        kes(12400)
    );
    repo.recover_dispatching_payouts(Timestamp(time().0 + 180))
        .await
        .unwrap();
    assert_eq!(
        service
            .make_payment(make("key", "q"))
            .await
            .unwrap()
            .state()
            .cdk_state(),
        CdkPaymentState::Pending
    );
    assert_eq!(server.count(), 0);
}

async fn resolve_sweep(
    repo: &Repository,
    p: &daraja_core::outbound::Payout,
    paid: bool,
    now: Timestamp,
) {
    use daraja_core::{
        identity::ReceiptId,
        inbound::EvidenceSource,
        outbound::{FinalOutcome, PayoutAttestation},
        repository::PayoutChange,
    };
    let operation = p.dispatch().operation_id();
    let (_, version) = repo.payout(operation).await.unwrap();
    let outcome = if paid {
        FinalOutcome::Paid {
            receipt: ReceiptId::new(&format!("RECEIPT{}", now.0)).unwrap(),
        }
    } else {
        FinalOutcome::DefinitivelyNotExecuted {
            evidence_record: id("synthetic-nonexecution"),
        }
    };
    repo.change_payout(
        operation,
        version,
        PayoutChange::Resolve(PayoutAttestation {
            source: EvidenceSource::ReviewedStatement(id("synthetic-sweep-evidence")),
            operation_id: operation.clone(),
            payload_hash: p.dispatch().hash(),
            amount: p.dispatch().payload().amount,
            destination: p.dispatch().payload().destination.clone(),
            outcome,
        }),
        now,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn sweep_recovery_cooldown_daily_budget_and_callback_timeout_share_outbound_safety() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let repo = Repository::open(&path).await.unwrap();
    seed_sweep(&repo).await;
    let server=Server::new(vec![Reply::Json(200,status_ack()),Reply::Json(200,json!({"ResponseCode":"0","OriginatorConversationID":"second-origin","ConversationID":"second-conv"}))]).await;
    let clock = LiquidityClock::new();
    let service = sweep_service(&repo, &server, &clock, true);
    let cfg = sweep_config();
    let first = service.sweep_if_needed(Some(&cfg)).await.unwrap().unwrap();
    let (base, task) = ingress(&repo).await;
    let url = base
        .join(&format!(
            "callbacks/b2b/timeout/{}",
            callback_secret(Endpoint::B2bTimeout)
        ))
        .unwrap();
    assert!(
        reqwest::Client::new()
            .post(url)
            .header("content-type", "application/json")
            .body(callback(Endpoint::B2bTimeout).to_string())
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    assert_eq!(
        repo.payout(first.dispatch().operation_id())
            .await
            .unwrap()
            .0
            .state(),
        OutboundState::UnknownInFlight
    );
    assert_eq!(
        repo.payout_callback_ids(first.dispatch().operation_id())
            .await
            .unwrap()
            .len(),
        1
    );
    resolve_sweep(&repo, &first, false, Timestamp(time().0 + 1)).await;
    assert_eq!(
        repo.float_status(AccountRole::Collection, Timestamp(time().0 + 1))
            .await
            .unwrap()
            .pending_liability,
        kes(0)
    );
    clock.set(60);
    super::super::liquidity_fixture::observe(
        &repo,
        AccountRole::Collection,
        "600001",
        "second-source",
        time().0 + 60,
        10000,
    )
    .await;
    assert!(service.sweep_if_needed(Some(&cfg)).await.is_err());
    assert_eq!(server.count(), 1);
    super::super::liquidity_fixture::observe(
        &repo,
        AccountRole::Disbursement,
        "600000",
        "after-failure",
        time().0 + 60,
        0,
    )
    .await;
    clock.set(120);
    let second = service.sweep_if_needed(Some(&cfg)).await.unwrap().unwrap();
    assert_eq!(server.count(), 2);
    resolve_sweep(&repo, &second, true, Timestamp(time().0 + 121)).await;
    assert_eq!(
        repo.float_status(AccountRole::Collection, Timestamp(time().0 + 121))
            .await
            .unwrap()
            .available,
        Some(kes(8900))
    );
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(time().0 + 121))
            .await
            .unwrap()
            .available,
        Some(kes(0))
    );
    clock.set(179);
    super::super::liquidity_fixture::observe(
        &repo,
        AccountRole::Collection,
        "600001",
        "source-only-refresh",
        time().0 + 179,
        8900,
    )
    .await;
    let mut larger_budget = cfg.clone();
    larger_budget.max_daily = kes(3300);
    assert!(matches!(
        service.sweep_if_needed(Some(&larger_budget)).await,
        Err(daraja_core::outbound::DispatchError::Repository(
            RepositoryError::Domain(Error::LiquidityUnavailable)
        ))
    ));
    assert_eq!(server.count(), 2);
    // Refresh both accounts after finality. A third transfer still exceeds the rolling daily budget.
    clock.set(240);
    super::super::liquidity_fixture::observe(
        &repo,
        AccountRole::Collection,
        "600001",
        "third-source",
        time().0 + 240,
        8900,
    )
    .await;
    super::super::liquidity_fixture::observe(
        &repo,
        AccountRole::Disbursement,
        "600000",
        "still-low",
        time().0 + 240,
        0,
    )
    .await;
    assert!(service.sweep_if_needed(Some(&cfg)).await.is_err());
    assert_eq!(server.count(), 2);
    clock.set(300);
    super::super::liquidity_fixture::observe(
        &repo,
        AccountRole::Disbursement,
        "600000",
        "utility-recovered",
        time().0 + 300,
        1000,
    )
    .await;
    assert!(service.sweep_if_needed(Some(&cfg)).await.unwrap().is_none());
    assert_eq!(server.count(), 2);
    assert!(
        repo.liquidity_alerts()
            .await
            .unwrap()
            .iter()
            .any(|(role, _, active, _)| *role == AccountRole::Disbursement && !active)
    );
    task.abort();
}

#[tokio::test]
async fn delayed_balance_callback_cannot_replace_newer_snapshot_or_release_quote_holds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let repo = Repository::open(&path).await.unwrap();
    setup(&repo).await;
    super::super::liquidity_fixture::observe(
        &repo,
        AccountRole::Disbursement,
        "600000",
        "initial",
        time().0,
        12500,
    )
    .await;
    quote(&repo, "held", "0712345678").await;
    let old = super::super::liquidity_fixture::prepare(
        &repo,
        AccountRole::Disbursement,
        "600000",
        "old",
        time().0 + 60,
    )
    .await;
    let new = super::super::liquidity_fixture::prepare(
        &repo,
        AccountRole::Disbursement,
        "600000",
        "new",
        time().0 + 120,
    )
    .await;
    super::super::liquidity_fixture::prepare(
        &repo,
        AccountRole::Collection,
        "600001",
        "current",
        time().0 + 120,
    )
    .await;
    let (base, task) = ingress(&repo).await;
    let url = base
        .join(&format!(
            "callbacks/balance/result/{}",
            callback_secret(Endpoint::BalanceResult)
        ))
        .unwrap();
    let server = Server::new(vec![]).await;
    let clock = LiquidityClock::new();
    clock.set(120);
    let monitor = BalanceMonitor {
        repository: repo.clone(),
        collection: balance_client(&server, &clock, &path, true, None),
        disbursement: balance_client(&server, &clock, &path, false, None),
        clock: clock.clone(),
        verifier: SyntheticBalanceVerifier,
        result_url: Secret::new("https://example.invalid/result"),
        timeout_url: Secret::new("https://example.invalid/timeout"),
    };
    for (query, available) in [(new, "125.00"), (old, "999999.00")] {
        let mut body = observation("unused", available);
        body["Result"]["OriginatorConversationID"] = json!(format!("origin-{}", query.as_str()));
        body["Result"]["ConversationID"] = json!(format!("conv-{}", query.as_str()));
        assert!(
            reqwest::Client::new()
                .post(url.clone())
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
                .await
                .unwrap()
                .status()
                .is_success()
        );
        monitor.run_once().await.unwrap();
        let status = repo
            .float_status(AccountRole::Disbursement, Timestamp(time().0 + 120))
            .await
            .unwrap();
        assert_eq!(status.balance, Some(kes(12500)));
        assert_eq!(status.available, Some(kes(100)));
        assert_eq!(status.quote_reservations, kes(12400));
    }
    assert_eq!(server.count(), 0);
    task.abort();
}

#[path = "support/liquidity_fixture.rs"]
mod liquidity_fixture;
use daraja_core::{
    Error, Timestamp,
    amount::{Amount, Unit},
    identity::{Id, ReceiptId, RequestKey, RequestPayload},
    inbound::EvidenceSource,
    liquidity::{AccountRole, LiquidityPolicy},
    outbound::{
        Ambiguity, CdkPaymentState, DispatchRecord, FinalOutcome, Payout, PayoutAttestation,
    },
    repository::{PayoutChange, Repository, RepositoryError},
    tariffs::{Band, Schedule, Tariffs},
    valuation::PaymentQuote,
};
use sqlx::{Connection, SqliteConnection};
use std::collections::BTreeMap;
fn id(s: &str) -> Id {
    Id::new(s).unwrap()
}
fn kes(n: u64) -> Amount {
    Amount::new(n, Unit::KesMinor)
}
async fn fixture() -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::open(dir.path().join("db")).await.unwrap();
    repo.configure_liquidity(LiquidityPolicy {
        collection: id("600001"),
        disbursement: id("600000"),
        minimum: kes(100),
        max_age_seconds: 100,
        poll_interval_seconds: 1,
    })
    .await
    .unwrap();
    (dir, repo)
}
fn quote(name: &str, now: u64, expiry: u64) -> PaymentQuote {
    let schedule = |version| {
        vec![Schedule {
            version: id(version),
            effective_from: Timestamp(0),
            effective_until: Timestamp(1000),
            bands: vec![Band {
                min_kes: 1,
                max_kes: 10000,
                fee: kes(10),
            }],
        }]
    };
    PaymentQuote::new(
        id(name),
        "0712345678",
        kes(100),
        Unit::KesMinor,
        &Tariffs::new(schedule("c"), schedule("b")).unwrap(),
        None,
        Timestamp(now),
        Timestamp(expiry),
    )
    .unwrap()
}
fn payout(q: &PaymentQuote, now: u64) -> (RequestKey, Payout) {
    let key = RequestKey {
        caller: id("mint"),
        operation: id("payout"),
        key: q.id().clone(),
    };
    let payload = RequestPayload {
        operation: key.operation.clone(),
        quote_id: q.id().clone(),
        destination: q.destination().clone(),
        amount: q.principal(),
        tariff_version: q.tariff().version.clone(),
        options: BTreeMap::new(),
    };
    let dispatch = DispatchRecord::new(
        id(&format!("p-{}", q.id().as_str())),
        payload,
        br#"{"PartyA":"600000"}"#.to_vec(),
        q.tariff().clone(),
        kes(110),
    )
    .unwrap()
    .with_quote(q.clone(), Timestamp(now))
    .unwrap();
    let p = Payout::dispatching(
        dispatch,
        id(&format!("caller-{}", q.id().as_str())),
        Timestamp(now),
        Timestamp(now + 1),
        Timestamp(now + 100),
    )
    .unwrap();
    (key, p)
}
async fn observe(repo: &Repository, name: &str, time: u64, amount: u64) {
    liquidity_fixture::observe(
        repo,
        AccountRole::Disbursement,
        "600000",
        name,
        time,
        amount,
    )
    .await;
}
fn is_domain<T: std::fmt::Debug>(result: Result<T, RepositoryError>, expected: Error) {
    assert!(matches!(result,Err(RepositoryError::Domain(e)) if e==expected));
}
#[tokio::test]
async fn unknown_stale_separate_accounts_and_exact_fee_coverage() {
    let (_dir, repo) = fixture().await;
    let q = quote("q", 10, 90);
    is_domain(
        repo.save_payment_quote(&q, Timestamp(10)).await,
        Error::LiquidityUnavailable,
    );
    liquidity_fixture::observe(
        &repo,
        AccountRole::Collection,
        "600001",
        "collection",
        10,
        1_000_000,
    )
    .await;
    is_domain(
        repo.save_payment_quote(&q, Timestamp(10)).await,
        Error::LiquidityUnavailable,
    );
    observe(&repo, "short-by-one", 10, 209).await;
    is_domain(
        repo.save_payment_quote(&q, Timestamp(10)).await,
        Error::InsufficientFloat,
    );
    observe(&repo, "enough", 11, 210).await;
    repo.save_payment_quote(&q, Timestamp(11)).await.unwrap();
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(11))
            .await
            .unwrap()
            .available,
        Some(kes(100))
    );
    is_domain(
        repo.save_payment_quote(&quote("stale", 111, 190), Timestamp(111))
            .await,
        Error::LiquidityUnavailable,
    );
    let metrics = repo.liquidity_metrics(Timestamp(111)).await.unwrap();
    assert!(metrics.contains("daraja_b2c_utility_balance_kes 2.10\n"));
    assert!(metrics.contains("daraja_c2b_collection_balance_kes 10000.00\n"));
    assert!(metrics.contains("daraja_liquidity_alert{account=\"disbursement\"} 1"));
    for forbidden in ["0712345678", "600000", "600001", "short-by-one"] {
        assert!(!metrics.contains(forbidden));
    }
    assert!(
        repo.float_status(AccountRole::Disbursement, Timestamp(111))
            .await
            .unwrap()
            .operator_instruction()
            .unwrap()
            .contains("bank/PesaLink")
    );
    observe(&repo, "recovery", 112, 210).await;
    repo.refresh_liquidity_alerts(Timestamp(112)).await.unwrap();
    assert!(
        !repo
            .float_status(AccountRole::Disbursement, Timestamp(112))
            .await
            .unwrap()
            .alert
    );
    repo.save_payment_quote(&quote("recovered", 112, 190), Timestamp(112))
        .await
        .unwrap();
}
#[tokio::test]
async fn concurrent_quote_holds_expire_and_survive_reopen_without_overcommit() {
    let (dir, repo) = fixture().await;
    observe(&repo, "float", 10, 430).await;
    let mut tasks = Vec::new();
    for i in 0..12 {
        let path = dir.path().join("db");
        tasks.push(tokio::spawn(async move {
            let r = Repository::open(path).await.unwrap();
            r.save_payment_quote(&quote(&format!("q{i}"), 10, 50), Timestamp(10))
                .await
        }));
    }
    let mut accepted = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(()) => accepted += 1,
            Err(RepositoryError::Domain(Error::InsufficientFloat)) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(accepted, 3);
    repo.close().await;
    let repo = Repository::open(dir.path().join("db")).await.unwrap();
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(49))
            .await
            .unwrap()
            .quote_reservations,
        kes(330)
    );
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(50))
            .await
            .unwrap()
            .quote_reservations,
        kes(0)
    );
    repo.save_payment_quote(&quote("after-expiry", 50, 90), Timestamp(50))
        .await
        .unwrap();
}
#[tokio::test]
async fn pending_and_paid_debits_survive_expiry_restart_and_out_of_order_observations() {
    let (dir, repo) = fixture().await;
    observe(&repo, "initial", 10, 320).await;
    let q = quote("q", 10, 20);
    repo.save_payment_quote(&q, Timestamp(10)).await.unwrap();
    let (key, p) = payout(&q, 11);
    let operation = repo.prepare_payout(&key, &p).await.unwrap();
    repo.change_payout(
        &operation,
        0,
        PayoutChange::Ambiguous(Ambiguity::Timeout),
        Timestamp(12),
    )
    .await
    .unwrap();
    repo.close().await;
    let repo = Repository::open(dir.path().join("db")).await.unwrap();
    let s = repo
        .float_status(AccountRole::Disbursement, Timestamp(21))
        .await
        .unwrap();
    assert_eq!(s.pending_liability, kes(110));
    assert_eq!(s.quote_reservations, kes(0));
    assert_eq!(s.available, Some(kes(210)));
    assert_eq!(
        repo.payout(&operation).await.unwrap().0.state().cdk_state(),
        CdkPaymentState::Pending
    );
    let late =
        liquidity_fixture::prepare(&repo, AccountRole::Disbursement, "600000", "late-old", 22)
            .await;
    let (_, version) = repo.payout(&operation).await.unwrap();
    repo.change_payout(
        &operation,
        version,
        PayoutChange::Resolve(PayoutAttestation {
            source: EvidenceSource::ReviewedStatement(id("synthetic")),
            operation_id: operation.clone(),
            payload_hash: p.dispatch().hash(),
            amount: q.principal(),
            destination: q.destination().clone(),
            outcome: FinalOutcome::Paid {
                receipt: ReceiptId::new("RECEIPT").unwrap(),
            },
        }),
        Timestamp(23),
    )
    .await
    .unwrap();
    // Paid releases the reservation ledger but must not replenish the old observation.
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(24))
            .await
            .unwrap()
            .available,
        Some(kes(210))
    );
    observe(&repo, "new-after-paid", 24, 210).await;
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(24))
            .await
            .unwrap()
            .pending_liability,
        kes(0)
    );
    liquidity_fixture::verify(&repo, AccountRole::Disbursement, "600000", late, 25, 10_000).await;
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(25))
            .await
            .unwrap()
            .available,
        Some(kes(210))
    );
    repo.save_payment_quote(&quote("second", 25, 90), Timestamp(25))
        .await
        .unwrap();
    is_domain(
        repo.save_payment_quote(&quote("third", 25, 90), Timestamp(25))
            .await,
        Error::InsufficientFloat,
    );
}
#[tokio::test]
async fn dispatch_rechecks_freshness_capacity_and_releases_only_definitive_failure() {
    let (dir, repo) = fixture().await;
    observe(&repo, "initial", 10, 320).await;
    let q = quote("q", 10, 190);
    repo.save_payment_quote(&q, Timestamp(10)).await.unwrap();
    observe(&repo, "drop", 11, 209).await;
    let (key, p) = payout(&q, 12);
    is_domain(
        repo.prepare_payout(&key, &p).await,
        Error::InsufficientFloat,
    );
    assert!(repo.payout(p.dispatch().operation_id()).await.is_err());
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(12))
            .await
            .unwrap()
            .quote_reservations,
        kes(110)
    );
    observe(&repo, "recover", 12, 320).await;
    let (key, p) = payout(&q, 112);
    is_domain(
        repo.prepare_payout(&key, &p).await,
        Error::LiquidityUnavailable,
    );
    observe(&repo, "fresh", 113, 320).await;
    let (key, p) = payout(&q, 114);
    let operation = repo.prepare_payout(&key, &p).await.unwrap();
    repo.change_payout(
        &operation,
        0,
        PayoutChange::Resolve(PayoutAttestation {
            source: EvidenceSource::ReviewedStatement(id("synthetic-non-execution")),
            operation_id: operation.clone(),
            payload_hash: p.dispatch().hash(),
            amount: q.principal(),
            destination: q.destination().clone(),
            outcome: FinalOutcome::DefinitivelyNotExecuted {
                evidence_record: id("verified"),
            },
        }),
        Timestamp(115),
    )
    .await
    .unwrap();
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(115))
            .await
            .unwrap()
            .available,
        Some(kes(320))
    );
    let mut raw = SqliteConnection::connect(&format!("sqlite:{}", dir.path().join("db").display()))
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_hold BEFORE INSERT ON quote_float_reservations BEGIN SELECT RAISE(ABORT,'fixture'); END").execute(&mut raw).await.unwrap();
    let rejected = quote("rollback", 115, 190);
    assert!(
        repo.save_payment_quote(&rejected, Timestamp(115))
            .await
            .is_err()
    );
    assert!(repo.payment_quote(rejected.id()).await.is_err());
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(115))
            .await
            .unwrap()
            .quote_reservations,
        kes(0)
    );
}

#[tokio::test]
async fn missing_policy_is_closed_and_quote_replay_never_extends_the_hold() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::open(dir.path().join("db")).await.unwrap();
    let q = quote("q", 10, 20);
    is_domain(
        repo.save_payment_quote(&q, Timestamp(10)).await,
        Error::LiquidityUnavailable,
    );
    liquidity_fixture::fund(&repo).await;
    repo.save_payment_quote(&q, Timestamp(10)).await.unwrap();
    repo.save_payment_quote(&q, Timestamp(30)).await.unwrap();
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(30))
            .await
            .unwrap()
            .quote_reservations,
        kes(0)
    );
    assert!(
        payout(&q, 19)
            .1
            .dispatch()
            .quote()
            .unwrap()
            .validate_dispatch(Timestamp(20))
            .is_err()
    );
}

#[tokio::test]
async fn stage13_upgrade_preserves_dispatch_callback_and_observation_rows() {
    use sqlx::sqlite::SqliteConnectOptions;
    let dir = tempfile::tempdir().unwrap();
    let migration_dir = dir.path().join("migrations");
    std::fs::create_dir(&migration_dir).unwrap();
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations")).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_str().unwrap();
        if name.ends_with(".sql") && name < "0009" {
            std::fs::copy(entry.path(), migration_dir.join(name)).unwrap();
        }
    }
    let path = dir.path().join("db");
    let mut raw = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    sqlx::migrate::Migrator::new(migration_dir.as_path())
        .await
        .unwrap()
        .run(&mut raw)
        .await
        .unwrap();
    let q = quote("legacy", 10, 90);
    let (_, p) = payout(&q, 11);
    let d = p.dispatch();
    sqlx::query("INSERT INTO payment_quotes VALUES(?,?)")
        .bind(q.id().as_str())
        .bind(serde_json::to_string(&q).unwrap())
        .execute(&mut raw)
        .await
        .unwrap();
    sqlx::query("INSERT INTO outbound_payouts(id,caller_request,state,payload_hash,dispatch,wire_body,next_check_at,manual_after,data) VALUES(?,?,'Dispatching',?,?,?,?,?,?)")
        .bind(d.operation_id().as_str()).bind(p.correlation().caller_request.as_str()).bind(d.hash().as_bytes().as_slice()).bind(serde_json::to_string(d).unwrap()).bind(d.wire_body()).bind(12).bind(111).bind(serde_json::to_string(&p).unwrap()).execute(&mut raw).await.unwrap();
    sqlx::query("INSERT INTO payout_dispatches VALUES(?,?,'https://example.invalid/payout','BusinessPayment')").bind(d.operation_id().as_str()).bind(q.id().as_str()).execute(&mut raw).await.unwrap();
    sqlx::query("INSERT INTO payout_dispatch_observations VALUES(?,'Acknowledged','{}',12)")
        .bind(d.operation_id().as_str())
        .execute(&mut raw)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO callback_inbox VALUES('callback','/callbacks/b2c/{secret}',NULL,12,'{}')",
    )
    .execute(&mut raw)
    .await
    .unwrap();
    sqlx::query("INSERT INTO payout_callback_links VALUES('callback',?)")
        .bind(d.operation_id().as_str())
        .execute(&mut raw)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO balance_reservations(payout_id,amount,unit) VALUES(?,'110','KesMinor')",
    )
    .bind(d.operation_id().as_str())
    .execute(&mut raw)
    .await
    .unwrap();
    raw.close().await.unwrap();
    let repo = Repository::open(&path).await.unwrap();
    assert_eq!(repo.payout(d.operation_id()).await.unwrap().0, p);
    assert_eq!(
        repo.payout_callback_ids(d.operation_id()).await.unwrap(),
        vec![id("callback")]
    );
    liquidity_fixture::fund(&repo).await;
    assert_eq!(
        repo.float_status(AccountRole::Disbursement, Timestamp(30))
            .await
            .unwrap()
            .pending_liability,
        kes(110)
    );
    let mut raw = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(&path)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    for table in [
        "payout_dispatches",
        "payout_dispatch_observations",
        "payout_callback_links",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(&mut raw)
                .await
                .unwrap(),
            1
        );
    }
    assert!(
        sqlx::query("UPDATE payout_dispatches SET command='BusinessPayBill'")
            .execute(&mut raw)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pragma_foreign_key_check")
            .fetch_one(&mut raw)
            .await
            .unwrap(),
        0
    );
}

#[path = "support/liquidity_fixture.rs"]
mod liquidity_fixture;
use daraja_core::{
    Error, Timestamp,
    amount::{Amount, TariffSnapshot, Unit},
    identity::{Id, ReceiptId, RequestKey, RequestPayload},
    inbound::{
        Allocation, DepositAttestation, DepositRail, DepositState, EvidenceSource, InboundQuote,
        InboundState, QuoteTerms, ReceiptObservation, StkAttempt, VerifiedDeposit,
    },
    outbound::{
        Ambiguity, CdkPaymentState, DispatchRecord, FinalOutcome, Payout, PayoutAttestation,
    },
    repository::{PayoutChange, Repository, RepositoryError, StkChange},
};
use sqlx::{Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{collections::BTreeMap, path::Path, process::Command};
use tempfile::TempDir;
fn id(s: &str) -> Id {
    Id::new(s).unwrap()
}
fn kes(n: u64) -> Amount {
    Amount::new(n, Unit::KesMinor)
}
fn tariff() -> TariffSnapshot {
    TariffSnapshot {
        version: id("v1"),
        fee: kes(2),
    }
}
fn quote(q: &str, account: &str) -> InboundQuote {
    InboundQuote::new(
        id(q),
        id(account),
        id("12345"),
        QuoteTerms {
            expected: kes(100),
            credit: kes(98),
            quote_expires_at: Timestamp(200),
            fx: None,
            tariff: tariff(),
            hold_until: Timestamp(0),
        },
    )
    .unwrap()
}
fn deposit(receipt: &str, q: Option<&str>, amount: u64) -> VerifiedDeposit {
    VerifiedDeposit::attest(DepositAttestation {
        payer: None,
        source: EvidenceSource::ReviewedStatement(id("synthetic-statement")),
        receipt: ReceiptId::new(receipt).unwrap(),
        quote_id: q.map(id),
        amount: kes(amount),
        shortcode: id("12345"),
        rail: DepositRail::C2b {
            account_ref: id("CDK9F2A1"),
        },
        received_at: Timestamp(30),
        verified_at: Timestamp(40),
    })
    .unwrap()
}
fn payout(op: &str, amount: u64) -> Payout {
    let p = RequestPayload {
        operation: id("payout"),
        quote_id: id("outquote"),
        destination: id("synthetic-destination"),
        amount: kes(amount),
        tariff_version: id("v1"),
        options: BTreeMap::new(),
    };
    Payout::dispatching(
        DispatchRecord::new(
            id(op),
            p,
            b"synthetic immutable wire body".to_vec(),
            tariff(),
            kes(amount + 2),
        )
        .unwrap(),
        id(&format!("caller-{op}")),
        Timestamp(1),
        Timestamp(35),
        Timestamp(100),
    )
    .unwrap()
}
fn key() -> RequestKey {
    RequestKey {
        caller: id("client"),
        operation: id("payout"),
        key: id("request-1"),
    }
}
async fn fixture() -> (TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let r = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    liquidity_fixture::fund(&r).await;
    (dir, r)
}
async fn raw(path: &Path) -> SqliteConnection {
    SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .foreign_keys(true),
    )
    .await
    .unwrap()
}
async fn count(c: &mut SqliteConnection, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(c)
        .await
        .unwrap()
}

#[tokio::test]
async fn migrations_reopen_and_connection_settings() {
    let (dir, r) = fixture().await;
    r.insert_quote(&quote("quote", "CDK9F2A1"), Timestamp(1))
        .await
        .unwrap();
    r.close().await;
    let r = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    assert_eq!(
        r.quote(&id("quote")).await.unwrap().0.state(),
        InboundState::Issued
    );
    let mut c = raw(&dir.path().join("ledger.sqlite")).await;
    assert_eq!(count(&mut c, "_sqlx_migrations").await, 15);
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA journal_mode")
            .fetch_one(&mut c)
            .await
            .unwrap(),
        "wal"
    );
    assert_eq!(count(&mut c, "audit_records").await, 3);
    r.close().await;
}

#[tokio::test]
async fn forged_first_callback_verification_crash_duplicate_and_settlement_replay() {
    let (dir, r) = fixture().await;
    r.insert_quote(&quote("quote", "CDK9F2A1"), Timestamp(1))
        .await
        .unwrap();
    let o = ReceiptObservation {
        observation_id: id("spoof"),
        claimed_receipt: ReceiptId::new("receipt").unwrap(),
        claimed_quote: Some(id("forged-quote")),
        claimed_amount: kes(999),
        rail: DepositRail::C2b {
            account_ref: id("WRONGREF"),
        },
        observed_at: Timestamp(10),
    };
    r.observe(&o).await.unwrap();
    r.observe(&o).await.unwrap();
    assert_eq!(r.observation(&id("spoof")).await.unwrap(), o);
    assert!(
        r.receipt(&ReceiptId::new("receipt").unwrap())
            .await
            .unwrap()
            .is_none()
    );
    let old = r.claim_job(Timestamp(10), 5).await.unwrap().unwrap();
    assert!(r.claim_job(Timestamp(14), 5).await.unwrap().is_none());
    r.close().await;
    let r = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    let resumed = r.claim_job(Timestamp(15), 5).await.unwrap().unwrap();
    assert!(resumed.generation > old.generation);
    let mut legitimate = o.clone();
    legitimate.observation_id = id("legitimate");
    legitimate.claimed_quote = Some(id("quote"));
    legitimate.claimed_amount = kes(100);
    legitimate.observed_at = Timestamp(20);
    r.observe(&legitimate).await.unwrap();

    assert!(matches!(
        r.finish_job(&old, Timestamp(16), None).await,
        Err(RepositoryError::LeaseLost)
    ));
    r.confirm(deposit("receipt", Some("quote"), 100), Some(Timestamp(50)))
        .await
        .unwrap();
    r.finish_job(&resumed, Timestamp(16), None).await.unwrap();
    assert!(matches!(
        r.confirm(deposit("RECEIPT", Some("quote"), 100), None)
            .await,
        Err(RepositoryError::Domain(Error::DuplicateReceipt))
    ));
    assert!(r.settle(&id("quote"), 1, Timestamp(60)).await.is_err());
    assert_eq!(r.events_after(0, 100).await.unwrap().len(), 1);
    let (sequence, event) = r.events_after(0, 100).await.unwrap().remove(0);
    assert_eq!(event.credit, kes(98));
    assert!(r.events_after(sequence, 100).await.unwrap().is_empty());
    r.close().await;
    let r = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    assert_eq!(
        r.quote(&id("quote")).await.unwrap().0.state(),
        InboundState::Paid
    );
    assert_eq!(
        r.receipt(&ReceiptId::new("receipt").unwrap())
            .await
            .unwrap()
            .unwrap()
            .state,
        DepositState::Settled
    );
    let mut c = raw(&dir.path().join("ledger.sqlite")).await;
    assert_eq!(count(&mut c, "receipt_observations").await, 2);
    assert_eq!(count(&mut c, "mpesa_receipts_ledger").await, 1);
    r.close().await;
}

#[tokio::test]
async fn competing_connections_cannot_allocate_or_settle_twice() {
    let (dir, a) = fixture().await;
    a.insert_quote(&quote("quote", "CDK9F2A1"), Timestamp(1))
        .await
        .unwrap();
    let b = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    let (x, y) = tokio::join!(
        a.confirm(deposit("first", Some("quote"), 100), Some(Timestamp(50))),
        b.confirm(deposit("second", Some("quote"), 100), Some(Timestamp(50)))
    );
    assert_eq!(usize::from(x.is_ok()) + usize::from(y.is_ok()), 1);
    // The distinct extra receipt is retained by confirmation without a second settlement.
    let retry = if x.is_ok() { "second" } else { "first" };
    b.confirm(deposit(retry, Some("quote"), 100), None)
        .await
        .unwrap();
    assert_eq!(
        b.receipt(&ReceiptId::new(retry).unwrap())
            .await
            .unwrap()
            .unwrap()
            .allocation,
        Allocation::Exception
    );
    assert_eq!(a.events_after(0, 10).await.unwrap().len(), 1);
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn concurrent_duplicate_delivery_and_topups() {
    let (dir, a) = fixture().await;
    a.insert_quote(&quote("quote", "CDK9F2A1"), Timestamp(1))
        .await
        .unwrap();
    let b = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    let (x, y) = tokio::join!(
        a.confirm(deposit("same", Some("quote"), 40), None),
        b.confirm(deposit("SAME", Some("quote"), 40), None)
    );
    assert_eq!(usize::from(x.is_ok()) + usize::from(y.is_ok()), 1);
    assert!(matches!(
        if x.is_ok() { y } else { x },
        Err(RepositoryError::Domain(Error::DuplicateReceipt))
    ));
    b.confirm(deposit("topup", Some("quote"), 60), Some(Timestamp(50)))
        .await
        .unwrap();
    assert_eq!(a.events_after(0, 10).await.unwrap()[0].1.receipts.len(), 2);
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn database_failure_rolls_back_all_settlement_writes() {
    let (dir, r) = fixture().await;
    r.insert_quote(&quote("quote", "CDK9F2A1"), Timestamp(1))
        .await
        .unwrap();
    let mut c = raw(&dir.path().join("ledger.sqlite")).await;
    sqlx::query("CREATE TRIGGER fail_event BEFORE INSERT ON settlement_events BEGIN SELECT RAISE(ABORT,'injected event failure'); END").execute(&mut c).await.unwrap();
    assert!(
        r.confirm(deposit("receipt", Some("quote"), 100), Some(Timestamp(50)))
            .await
            .is_err()
    );
    assert_eq!(
        r.quote(&id("quote")).await.unwrap().0.state(),
        InboundState::Issued
    );
    assert_eq!(count(&mut c, "mpesa_receipts_ledger").await, 0);
    assert_eq!(count(&mut c, "settlement_events").await, 0);
    assert_eq!(count(&mut c, "audit_records").await, 3);
    sqlx::query("DROP TRIGGER fail_event")
        .execute(&mut c)
        .await
        .unwrap();
    r.confirm(deposit("receipt", Some("quote"), 100), Some(Timestamp(50)))
        .await
        .unwrap();
    assert_eq!(r.events_after(0, 10).await.unwrap().len(), 1);
    r.close().await;
}

#[tokio::test]
async fn invalid_binding_cas_and_overpayment_do_not_mint() {
    let (_dir, r) = fixture().await;
    r.insert_quote(&quote("quote", "CDK9F2A1"), Timestamp(1))
        .await
        .unwrap();
    let mut e = deposit("bad", Some("quote"), 100).evidence().clone();
    e.shortcode = id("wrong");
    assert!(
        r.confirm(VerifiedDeposit::attest(e).unwrap(), None)
            .await
            .is_err()
    );
    assert!(
        r.receipt(&ReceiptId::new("bad").unwrap())
            .await
            .unwrap()
            .is_none()
    );
    r.confirm(deposit("first", Some("quote"), 100), None)
        .await
        .unwrap();
    assert!(matches!(
        r.settle(&id("quote"), 0, Timestamp(50)).await,
        Err(RepositoryError::Conflict)
    ));
    assert!(r.events_after(0, 10).await.unwrap().is_empty());
    r.confirm(deposit("extra", Some("quote"), 100), None)
        .await
        .unwrap();
    assert!(r.settle(&id("quote"), 2, Timestamp(50)).await.is_err());
    r.confirm(deposit("orphan", None, 100), None).await.unwrap();
    assert_eq!(
        r.receipt(&ReceiptId::new("orphan").unwrap())
            .await
            .unwrap()
            .unwrap()
            .state,
        DepositState::OrphanUnclaimed
    );
    r.close().await;
}

#[tokio::test]
async fn idempotent_payout_reopen_immutable_dispatch_and_pending_reservations() {
    let (dir, r) = fixture().await;
    let p = payout("operation", 100);
    assert_eq!(r.prepare_payout(&key(), &p).await.unwrap(), id("operation"));
    assert_eq!(
        r.prepare_payout(&key(), &payout("fresh-proposal", 100))
            .await
            .unwrap(),
        id("operation")
    );
    assert!(matches!(
        r.prepare_payout(&key(), &payout("different", 101)).await,
        Err(RepositoryError::Domain(Error::IdempotencyConflict))
    ));
    r.close().await;
    let r = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    assert_eq!(r.payout(&id("operation")).await.unwrap().0, p);
    r.change_payout(
        &id("operation"),
        0,
        PayoutChange::Ambiguous(Ambiguity::WorkerRecovery),
        Timestamp(36),
    )
    .await
    .unwrap();
    assert!(matches!(
        r.change_payout(&id("operation"), 0, PayoutChange::Escalate, Timestamp(100))
            .await,
        Err(RepositoryError::Conflict)
    ));
    r.change_payout(&id("operation"), 1, PayoutChange::Escalate, Timestamp(100))
        .await
        .unwrap();
    let pending = r.payout(&id("operation")).await.unwrap().0;
    assert_eq!(pending.state().cdk_state(), CdkPaymentState::Pending);
    assert_eq!(pending.dispatch(), p.dispatch());
    let mut c = raw(&dir.path().join("ledger.sqlite")).await;
    assert!(
        sqlx::query("UPDATE outbound_payouts SET wire_body=x'00'")
            .execute(&mut c)
            .await
            .is_err()
    );
    let released: Option<i64> = sqlx::query_scalar("SELECT released_at FROM balance_reservations")
        .fetch_one(&mut c)
        .await
        .unwrap();
    assert!(released.is_none());
    let evidence = PayoutAttestation {
        source: EvidenceSource::ReviewedStatement(id("synthetic")),
        operation_id: id("operation"),
        payload_hash: p.dispatch().hash(),
        amount: kes(100),
        destination: p.dispatch().payload().destination.clone(),
        outcome: FinalOutcome::Paid {
            receipt: ReceiptId::new("paid").unwrap(),
        },
    };
    r.change_payout(
        &id("operation"),
        2,
        PayoutChange::Resolve(evidence),
        Timestamp(101),
    )
    .await
    .unwrap();
    let released: Option<i64> = sqlx::query_scalar("SELECT released_at FROM balance_reservations")
        .fetch_one(&mut c)
        .await
        .unwrap();
    assert_eq!(released, Some(101));
    assert!(
        r.change_payout(
            &id("operation"),
            3,
            PayoutChange::Ambiguous(Ambiguity::Timeout),
            Timestamp(102)
        )
        .await
        .is_err()
    );
    r.close().await;
}

#[tokio::test]
async fn competing_request_keys_and_job_leases_are_atomic() {
    let (dir, a) = fixture().await;
    let b = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    let p1 = payout("first", 100);
    let p2 = payout("second", 100);
    let k = key();
    let (x, y) = tokio::join!(a.prepare_payout(&k, &p1), b.prepare_payout(&k, &p2));
    assert_eq!(x.unwrap(), y.unwrap());
    let (x, y) = tokio::join!(a.claim_job(Timestamp(35), 5), b.claim_job(Timestamp(35), 5));
    assert_eq!(
        usize::from(x.unwrap().is_some()) + usize::from(y.unwrap().is_some()),
        1
    );
    assert!(a.claim_job(Timestamp(36), 301).await.is_err());
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn account_attempt_and_provider_correlations_are_database_unique() {
    let (dir, r) = fixture().await;
    r.insert_quote(&quote("quote", "CDK9F2A1"), Timestamp(1))
        .await
        .unwrap();
    assert!(
        r.insert_quote(&quote("other", "CDK9F2A1"), Timestamp(1))
            .await
            .is_err()
    );
    r.insert_quote(&quote("other", "CDK9F2A2"), Timestamp(1))
        .await
        .unwrap();
    for (q, a, caller) in [("quote", "a1", "c1"), ("other", "a2", "c2")] {
        r.change_stk(
            &id(q),
            0,
            StkChange::Start(StkAttempt::new(id(a), id(caller), Timestamp(1)).unwrap()),
            Timestamp(1),
        )
        .await
        .unwrap();
    }
    r.change_stk(
        &id("quote"),
        1,
        StkChange::Acknowledge {
            attempt: id("a1"),
            merchant: id("m1"),
            checkout: id("checkout"),
        },
        Timestamp(2),
    )
    .await
    .unwrap();
    assert!(
        r.change_stk(
            &id("other"),
            1,
            StkChange::Acknowledge {
                attempt: id("a2"),
                merchant: id("m2"),
                checkout: id("checkout")
            },
            Timestamp(2)
        )
        .await
        .is_err()
    );
    assert!(
        r.quote(&id("other")).await.unwrap().0.attempts()[0]
            .checkout_request()
            .is_none()
    );
    let mut k = key();
    r.prepare_payout(&k, &payout("one", 100)).await.unwrap();
    k.key = id("second");
    r.prepare_payout(&k, &payout("two", 100)).await.unwrap();
    r.change_payout(
        &id("one"),
        0,
        PayoutChange::Acknowledge {
            originator: id("proxy"),
            conversation: id("provider"),
        },
        Timestamp(3),
    )
    .await
    .unwrap();
    assert!(
        r.change_payout(
            &id("two"),
            0,
            PayoutChange::Acknowledge {
                originator: id("proxy2"),
                conversation: id("provider")
            },
            Timestamp(3)
        )
        .await
        .is_err()
    );
    let mut c = raw(&dir.path().join("ledger.sqlite")).await;
    assert!(
        sqlx::query("INSERT INTO unreconciled_deposits(receipt,reason) VALUES('MISSING','ORPHAN')")
            .execute(&mut c)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM audit_records")
            .execute(&mut c)
            .await
            .is_err()
    );
    r.close().await;
}

#[tokio::test]
async fn expired_job_is_retried_and_stale_worker_is_fenced() {
    let (_dir, r) = fixture().await;
    liquidity_fixture::fund(&r).await;
    r.prepare_payout(&key(), &payout("operation", 100))
        .await
        .unwrap();
    let a = r.claim_job(Timestamp(35), 5).await.unwrap().unwrap();
    assert!(matches!(
        r.finish_job(&a, Timestamp(40), None).await,
        Err(RepositoryError::LeaseLost)
    ));
    let b = r.claim_job(Timestamp(40), 5).await.unwrap().unwrap();
    r.finish_job(&b, Timestamp(41), Some(Timestamp(60)))
        .await
        .unwrap();
    assert!(r.claim_job(Timestamp(59), 5).await.unwrap().is_none());
    let c = r.claim_job(Timestamp(60), 5).await.unwrap().unwrap();
    r.finish_job(&c, Timestamp(61), None).await.unwrap();
    assert!(r.claim_job(Timestamp(100), 5).await.unwrap().is_none());
    r.close().await;
}

#[tokio::test]
async fn crash_child() {
    let Ok(path) = std::env::var("DARAJA_SQLITE_CRASH_PATH") else {
        return;
    };
    let r = Repository::open(&path).await.unwrap();
    liquidity_fixture::fund(&r).await;
    r.prepare_payout(&key(), &payout("operation", 100))
        .await
        .unwrap();
    if std::env::var("DARAJA_SQLITE_CRASH_MODE").unwrap() == "uncommitted" {
        let mut c = raw(Path::new(&path)).await;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut c)
            .await
            .unwrap();
        sqlx::query("UPDATE outbound_payouts SET state='Paid'")
            .execute(&mut c)
            .await
            .unwrap();
        std::process::exit(71);
    }
    // Abrupt exit: no pool shutdown or Rust destructors, and no transport invocation.
    std::process::exit(71);
}

#[tokio::test]
async fn process_exit_after_commit_before_dispatch_and_uncommitted_write_recover() {
    for mode in ["committed", "uncommitted"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.sqlite");
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_child", "--nocapture"])
            .env("DARAJA_SQLITE_CRASH_PATH", &path)
            .env("DARAJA_SQLITE_CRASH_MODE", mode)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(71));
        let r = Repository::open(&path).await.unwrap();
        let (p, _) = r.payout(&id("operation")).await.unwrap();
        assert_eq!(p.state().cdk_state(), CdkPaymentState::Pending);
        assert_eq!(p.dispatch(), payout("operation", 100).dispatch());
        let mut c = raw(&path).await;
        let row = sqlx::query("SELECT state FROM outbound_payouts")
            .fetch_one(&mut c)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("state"), "Dispatching");
        assert_eq!(count(&mut c, "balance_reservations").await, 1);
        assert_eq!(count(&mut c, "request_bindings").await, 1);
        let job = r.claim_job(Timestamp(35), 5).await.unwrap().unwrap();
        assert_eq!(job.kind, "RECONCILE_PAYOUT");
        r.close().await;
    }
}

#[tokio::test]
async fn ordered_upgrade_preserves_rows_and_checksum_tampering_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let migration_dir = dir.path().join("migrations");
    std::fs::create_dir(&migration_dir).unwrap();
    std::fs::write(
        migration_dir.join("0001_accounting.sql"),
        include_str!("../migrations/0001_accounting.sql"),
    )
    .unwrap();
    let path = dir.path().join("upgrade.sqlite");
    let mut c = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::migrate::Migrator::new(migration_dir.as_path())
        .await
        .unwrap()
        .run(&mut c)
        .await
        .unwrap();
    let q = quote("quote", "CDK9F2A1");
    sqlx::query("INSERT INTO inbound_quotes(id,account_ref,state,data) VALUES('quote','CDK9F2A1','Issued',?)").bind(serde_json::to_string(&q).unwrap()).execute(&mut c).await.unwrap();
    c.close().await.unwrap();
    let r = Repository::open(&path).await.unwrap();
    assert_eq!(r.quote(&id("quote")).await.unwrap().0, q);
    r.close().await;
    let mut c = raw(&path).await;
    assert_eq!(count(&mut c, "_sqlx_migrations").await, 15);
    sqlx::query("UPDATE _sqlx_migrations SET checksum=x'00' WHERE version=1")
        .execute(&mut c)
        .await
        .unwrap();
    c.close().await.unwrap();
    assert!(Repository::open(&path).await.is_err());
}

#[tokio::test]
async fn writer_child() {
    let Ok(path) = std::env::var("DARAJA_SQLITE_WRITER_PATH") else {
        return;
    };
    let op = std::env::var("DARAJA_SQLITE_WRITER_ID").unwrap();
    let r = Repository::open(&path).await.unwrap();
    r.prepare_payout(&key(), &payout(&op, 100)).await.unwrap();
    r.close().await;
}

#[tokio::test]
async fn separate_process_writers_bind_one_request_and_reservation() {
    let (dir, r) = fixture().await;
    r.close().await;
    let path = dir.path().join("ledger.sqlite");
    let exe = std::env::current_exe().unwrap();
    let mut children = Vec::new();
    for op in ["first", "second"] {
        children.push(
            Command::new(&exe)
                .args(["--exact", "writer_child"])
                .env("DARAJA_SQLITE_WRITER_PATH", &path)
                .env("DARAJA_SQLITE_WRITER_ID", op)
                .spawn()
                .unwrap(),
        );
    }
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let mut c = raw(&path).await;
    for table in [
        "outbound_payouts",
        "request_bindings",
        "balance_reservations",
        "durable_jobs",
    ] {
        assert_eq!(count(&mut c, table).await, 1);
    }
}

#[tokio::test]
async fn unsigned_money_roundtrips_and_overflow_rolls_back() {
    let (dir, r) = fixture().await;
    let mut terms = quote("quote", "CDK9F2A1").terms().clone();
    terms.expected = kes(u64::MAX);
    terms.credit = kes(u64::MAX - 2);
    let q = InboundQuote::new(id("quote"), id("CDK9F2A1"), id("12345"), terms).unwrap();
    r.insert_quote(&q, Timestamp(1)).await.unwrap();
    r.confirm(deposit("first", Some("quote"), u64::MAX - 1), None)
        .await
        .unwrap();
    r.close().await;
    let r = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    assert_eq!(
        r.quote(&id("quote")).await.unwrap().0.terms().expected,
        kes(u64::MAX)
    );
    assert!(matches!(
        r.confirm(deposit("overflow", Some("quote"), 2), None).await,
        Err(RepositoryError::Domain(Error::Overflow))
    ));
    assert!(
        r.receipt(&ReceiptId::new("overflow").unwrap())
            .await
            .unwrap()
            .is_none()
    );
    r.confirm(deposit("last", Some("quote"), 1), Some(Timestamp(50)))
        .await
        .unwrap();
    assert_eq!(
        r.events_after(0, 10).await.unwrap()[0].1.credit,
        kes(u64::MAX - 2)
    );
    r.close().await;
}

#[tokio::test]
async fn persisted_fx_and_hold_deadlines_still_gate_settlement_after_reopen() {
    let (dir, r) = fixture().await;
    let mut terms = quote("quote", "CDK9F2A1").terms().clone();
    terms.fx = Some(daraja_core::amount::FxSnapshot {
        source: id("synthetic-fx"),
        numerator: 3,
        denominator: 2,
        expires_at: Timestamp(60),
    });
    terms.credit = Amount::new(147, Unit::Sat);
    terms.hold_until = Timestamp(55);
    let q = InboundQuote::new(id("quote"), id("CDK9F2A1"), id("12345"), terms).unwrap();
    r.insert_quote(&q, Timestamp(1)).await.unwrap();
    r.confirm(deposit("first", Some("quote"), 100), None)
        .await
        .unwrap();
    r.close().await;
    let r = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    assert!(matches!(
        r.settle(&id("quote"), 1, Timestamp(54)).await,
        Err(RepositoryError::Domain(Error::SettlementHeld))
    ));
    assert!(matches!(
        r.settle(&id("quote"), 1, Timestamp(60)).await,
        Err(RepositoryError::Domain(Error::FxExpired))
    ));
    assert!(r.events_after(0, 10).await.unwrap().is_empty());
    assert_eq!(
        r.settle(&id("quote"), 1, Timestamp(55))
            .await
            .unwrap()
            .credit,
        Amount::new(147, Unit::Sat)
    );
    r.close().await;
}

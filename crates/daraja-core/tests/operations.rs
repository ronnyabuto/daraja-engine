#[path = "support/liquidity_fixture.rs"]
mod liquidity_fixture;

use daraja_core::{
    Timestamp,
    amount::{Amount, TariffSnapshot, Unit},
    crypto::Secret,
    identity::{Id, ReceiptId, RequestKey, RequestPayload},
    inbound::EvidenceSource,
    operations::{OperatorCredential, OperatorError, OperatorRole, OperatorService, ResolvePayout},
    outbound::{
        CdkPaymentState, DispatchRecord, FinalOutcome, OutboundState, Payout, PayoutAttestation,
    },
    repository::{PayoutChange, Repository, RepositoryError},
};
use sqlx::{Connection, Row, SqliteConnection};
use std::collections::BTreeMap;

fn id(s: &str) -> Id {
    Id::new(s).unwrap()
}
fn kes(n: u64) -> Amount {
    Amount::new(n, Unit::KesMinor)
}
fn token() -> Secret {
    Secret::new("synthetic-operator-token-0000000000000000")
}
fn observer() -> Secret {
    Secret::new("synthetic-observer-token-0000000000000000")
}
fn service(r: &Repository) -> OperatorService {
    OperatorService::new(
        r.clone(),
        vec![
            OperatorCredential::new(id("operator-alice"), &token(), OperatorRole::Reconciler)
                .unwrap(),
            OperatorCredential::new(id("auditor-bob"), &observer(), OperatorRole::Observer)
                .unwrap(),
        ],
    )
    .unwrap()
}
async fn fixture() -> (tempfile::TempDir, Repository, ResolvePayout) {
    let dir = tempfile::tempdir().unwrap();
    let r = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    liquidity_fixture::fund(&r).await;
    let payload = RequestPayload {
        operation: id("payout"),
        quote_id: id("outquote"),
        destination: id("synthetic-private-destination"),
        amount: kes(10000),
        tariff_version: id("v1"),
        options: BTreeMap::new(),
    };
    let p = Payout::dispatching(
        DispatchRecord::new(
            id("payout-1"),
            payload,
            b"synthetic-secret-wire-body".to_vec(),
            TariffSnapshot {
                version: id("v1"),
                fee: kes(100),
            },
            kes(10100),
        )
        .unwrap(),
        id("caller-1"),
        Timestamp(1),
        Timestamp(35),
        Timestamp(100),
    )
    .unwrap();
    r.prepare_payout(
        &RequestKey {
            caller: id("application"),
            operation: id("payout"),
            key: id("request-1"),
        },
        &p,
    )
    .await
    .unwrap();
    r.change_payout(&id("payout-1"), 0, PayoutChange::Escalate, Timestamp(100))
        .await
        .unwrap();
    let request = ResolvePayout {
        command_id: id("resolution-1"),
        payout_id: id("payout-1"),
        expected_version: 1,
        reason: id("case-17-reviewed"),
        evidence_ref: id("portal-record-17"),
        evidence: PayoutAttestation {
            source: EvidenceSource::ReviewedStatement(id("portal-record-17")),
            operation_id: id("payout-1"),
            payload_hash: p.dispatch().hash(),
            amount: kes(10000),
            destination: p.dispatch().payload().destination.clone(),
            outcome: FinalOutcome::Paid {
                receipt: ReceiptId::new("ACTUAL17").unwrap(),
            },
        },
    };
    (dir, r, request)
}
async fn connection(dir: &tempfile::TempDir) -> SqliteConnection {
    SqliteConnection::connect(&format!(
        "sqlite://{}",
        dir.path().join("ledger.sqlite").display()
    ))
    .await
    .unwrap()
}

#[tokio::test]
async fn operator_auth_and_redaction_preserve_pending() {
    let (_dir, r, request) = fixture().await;
    let s = service(&r);
    assert!(matches!(
        s.resolve_payout(&Secret::new("application-token"), &request, Timestamp(101))
            .await,
        Err(OperatorError::Unauthorized)
    ));
    assert!(matches!(
        s.resolve_payout(&observer(), &request, Timestamp(101))
            .await,
        Err(OperatorError::Forbidden)
    ));
    assert!(matches!(
        s.payout(&Secret::new("wrong"), &request.payout_id).await,
        Err(OperatorError::Unauthorized)
    ));
    let view = s.payout(&observer(), &request.payout_id).await.unwrap();
    assert_eq!(view.cdk_state, CdkPaymentState::Pending);
    let text = serde_json::to_string(&view).unwrap();
    assert!(!text.contains("destination"));
    assert!(!text.contains("secret-wire"));
    assert!(!text.contains("portal-record"));
}

#[tokio::test]
async fn manual_resolution_drill_replays_after_operator_restart() {
    let (dir, r, request) = fixture().await;
    let s = service(&r);
    assert_eq!(
        s.payout(&token(), &request.payout_id)
            .await
            .unwrap()
            .cdk_state,
        CdkPaymentState::Pending
    );
    let mut invalid = request.clone();
    invalid.evidence.amount = kes(9900);
    assert!(
        s.resolve_payout(&token(), &invalid, Timestamp(101))
            .await
            .is_err()
    );
    assert_eq!(
        s.payout(&token(), &request.payout_id)
            .await
            .unwrap()
            .cdk_state,
        CdkPaymentState::Pending
    );
    let paid = s
        .resolve_payout(&token(), &request, Timestamp(102))
        .await
        .unwrap();
    assert_eq!(paid.cdk_state, CdkPaymentState::Paid);
    drop(s);
    r.close().await;
    let reopened = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    assert_eq!(
        service(&reopened)
            .resolve_payout(&token(), &request, Timestamp(999))
            .await
            .unwrap(),
        paid
    );
    assert_eq!(
        reopened
            .payout(&request.payout_id)
            .await
            .unwrap()
            .0
            .final_evidence()
            .unwrap()
            .outcome,
        request.evidence.outcome
    );
    let mut c = connection(&dir).await;
    let row = sqlx::query(
        "SELECT actor,reason,evidence_ref,expected_version,outcome,request FROM operator_commands",
    )
    .fetch_one(&mut c)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("actor"), "operator-alice");
    assert_eq!(row.get::<String, _>("reason"), "case-17-reviewed");
    assert_eq!(row.get::<String, _>("evidence_ref"), "portal-record-17");
    assert_eq!(row.get::<i64, _>("expected_version"), 1);
    assert_eq!(row.get::<String, _>("outcome"), "APPLIED");
    assert_eq!(
        row.get::<String, _>("request"),
        serde_json::to_string(&request).unwrap()
    );
    assert!(
        sqlx::query("DELETE FROM operator_commands")
            .execute(&mut c)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM operator_commands")
            .fetch_one(&mut c)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM balance_reservations WHERE released_at IS NULL"
        )
        .fetch_one(&mut c)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn repeated_concurrent_and_stale_commands_cannot_resolve_twice() {
    let (_dir, r, request) = fixture().await;
    let s1 = service(&r);
    let s2 = service(&r);
    let t = token();
    let (a, b) = tokio::join!(
        s1.resolve_payout(&t, &request, Timestamp(101)),
        s2.resolve_payout(&t, &request, Timestamp(101))
    );
    assert_eq!(a.unwrap(), b.unwrap());
    let mut stale = request.clone();
    stale.command_id = id("resolution-2");
    assert!(matches!(
        s1.resolve_payout(&token(), &stale, Timestamp(102)).await,
        Err(OperatorError::Repository(RepositoryError::Conflict))
    ));
    let mut changed = request.clone();
    changed.reason = id("changed-reason");
    assert!(
        s1.resolve_payout(&token(), &changed, Timestamp(102))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn late_success_after_definitive_failure_records_conflict_without_rewriting_money() {
    let (dir, r, mut request) = fixture().await;
    let s = service(&r);
    request.evidence.outcome = FinalOutcome::DefinitivelyNotExecuted {
        evidence_record: request.evidence_ref.clone(),
    };
    assert_eq!(
        s.resolve_payout(&token(), &request, Timestamp(101))
            .await
            .unwrap()
            .cdk_state,
        CdkPaymentState::Failed
    );
    let mut late = request.clone();
    late.command_id = id("late-success-review");
    late.expected_version = 2;
    late.evidence.outcome = FinalOutcome::Paid {
        receipt: ReceiptId::new("LATE18").unwrap(),
    };
    assert!(matches!(
        s.resolve_payout(&token(), &late, Timestamp(102)).await,
        Err(OperatorError::ContradictoryEvidence)
    ));
    assert!(matches!(
        s.resolve_payout(&token(), &late, Timestamp(103)).await,
        Err(OperatorError::ContradictoryEvidence)
    ));
    assert_eq!(
        s.payout(&token(), &request.payout_id).await.unwrap().state,
        OutboundState::Failed
    );
    let mut c = connection(&dir).await;
    let observed: String =
        sqlx::query_scalar("SELECT request FROM operator_commands WHERE outcome='CONTRADICTORY'")
            .fetch_one(&mut c)
            .await
            .unwrap();
    assert!(observed.contains("LATE18"));
}

#[tokio::test]
async fn failed_audit_write_rolls_back_resolution_and_reservation_release() {
    let (dir, r, request) = fixture().await;
    let mut c = connection(&dir).await;
    sqlx::query("CREATE TRIGGER fail_operator_audit BEFORE INSERT ON operator_commands BEGIN SELECT RAISE(ABORT,'synthetic fault'); END").execute(&mut c).await.unwrap();
    let s = service(&r);
    assert!(
        s.resolve_payout(&token(), &request, Timestamp(101))
            .await
            .is_err()
    );
    assert_eq!(
        s.payout(&token(), &request.payout_id)
            .await
            .unwrap()
            .cdk_state,
        CdkPaymentState::Pending
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM balance_reservations WHERE released_at IS NULL"
        )
        .fetch_one(&mut c)
        .await
        .unwrap(),
        1
    );
    sqlx::query("DROP TRIGGER fail_operator_audit")
        .execute(&mut c)
        .await
        .unwrap();
    assert!(
        s.resolve_payout(&token(), &request, Timestamp(102))
            .await
            .is_ok()
    );
}

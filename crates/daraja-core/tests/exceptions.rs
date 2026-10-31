use daraja_core::{
    Error, Timestamp,
    amount::{Amount, FxSnapshot, TariffSnapshot, Unit},
    identity::{Id, ReceiptId},
    inbound::{
        Allocation, DepositAttestation, DepositRail, DepositState, EvidenceSource, InboundQuote,
        InboundState, QuoteTerms, VerifiedDeposit, VerifiedPayer,
    },
    repository::{DepositPolicy, RefundAuthorization, Repository, RepositoryError},
};
use sqlx::{Connection, SqliteConnection};
fn id(s: &str) -> Id {
    Id::new(s).unwrap()
}
fn receipt(s: &str) -> ReceiptId {
    ReceiptId::new(s).unwrap()
}
fn kes(n: u64) -> Amount {
    Amount::new(n, Unit::KesMinor)
}
fn fee(n: u64) -> TariffSnapshot {
    TariffSnapshot {
        version: id("synthetic-b2c-v1"),
        fee: kes(n),
    }
}
fn quote(name: &str, sat: bool) -> InboundQuote {
    InboundQuote::new(
        id(name),
        id(name),
        id("12345"),
        QuoteTerms {
            expected: kes(10000),
            credit: Amount::new(9900, if sat { Unit::Sat } else { Unit::KesMinor }),
            tariff: fee(100),
            quote_expires_at: Timestamp(100),
            hold_until: Timestamp(0),
            fx: sat.then(|| FxSnapshot {
                source: id("synthetic-fx"),
                numerator: 1,
                denominator: 1,
                expires_at: Timestamp(100),
            }),
        },
    )
    .unwrap()
}
fn evidence(r: &str, q: Option<&str>, n: u64, received: u64, verified: u64) -> DepositAttestation {
    DepositAttestation {
        payer: None,
        source: EvidenceSource::ReviewedStatement(id("synthetic-statement")),
        receipt: receipt(r),
        quote_id: q.map(id),
        amount: kes(n),
        shortcode: id("12345"),
        rail: DepositRail::C2b {
            account_ref: id(q.unwrap_or("TYPO0001")),
        },
        received_at: Timestamp(received),
        verified_at: Timestamp(verified),
    }
}
fn deposit(r: &str, q: Option<&str>, n: u64) -> VerifiedDeposit {
    VerifiedDeposit::attest(evidence(r, q, n, 10, 20)).unwrap()
}
fn authorization(receipts: &[&str]) -> RefundAuthorization {
    RefundAuthorization {
        operator: id("authenticated-operator"),
        owner: id("reviewed-owner"),
        evidence: id("reviewed-original-payer-and-destination"),
        destination: id("reviewed-destination"),
        receipts: receipts.iter().map(|r| receipt(r)).collect(),
    }
}
async fn fixture() -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    (dir, repo)
}
async fn count(dir: &tempfile::TempDir, table: &str) -> i64 {
    let mut c = SqliteConnection::connect(&format!(
        "sqlite://{}",
        dir.path().join("ledger.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(&mut c)
        .await
        .unwrap()
}
async fn settle(repo: &Repository, q: &str, now: u64) -> Result<(), RepositoryError> {
    let (_, v) = repo.quote(&id(q)).await?;
    repo.settle(&id(q), v, Timestamp(now)).await.map(|_| ())
}
#[tokio::test]
async fn distinct_verified_topups_credit_once_after_reopen() {
    let (dir, r) = fixture().await;
    r.insert_quote(&quote("TOPUP001", false), Timestamp(1))
        .await
        .unwrap();
    assert_eq!(
        r.confirm(deposit("PART1", Some("TOPUP001"), 4000), None)
            .await
            .unwrap(),
        DepositState::UnderpaidHeld
    );
    assert!(settle(&r, "TOPUP001", 21).await.is_err());
    r.close().await;
    let r = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    r.confirm(
        deposit("PART2", Some("TOPUP001"), 6000),
        Some(Timestamp(21)),
    )
    .await
    .unwrap();
    assert!(matches!(
        r.confirm(deposit("part2", Some("TOPUP001"), 6000), None)
            .await,
        Err(RepositoryError::Domain(Error::DuplicateReceipt))
    ));
    assert_eq!(count(&dir, "settlement_events").await, 1);
    assert_eq!(
        r.receipt(&receipt("PART1")).await.unwrap().unwrap().state,
        DepositState::Settled
    );
    assert!(settle(&r, "TOPUP001", 22).await.is_err());
}
#[tokio::test]
async fn overpayment_is_held_and_refund_is_durable_net_of_fee() {
    let (dir, r) = fixture().await;
    r.insert_quote(&quote("OVER0001", false), Timestamp(1))
        .await
        .unwrap();
    r.confirm(deposit("OVER", Some("OVER0001"), 11000), None)
        .await
        .unwrap();
    assert_eq!(
        r.quote(&id("OVER0001")).await.unwrap().0.state(),
        InboundState::OverpaidHeld
    );
    assert!(settle(&r, "OVER0001", 21).await.is_err());
    let request = r
        .request_refund(
            &id("refund"),
            authorization(&["OVER"]),
            fee(100),
            Timestamp(21),
        )
        .await
        .unwrap();
    assert_eq!(request.net, kes(10900));
    r.close().await;
    let r = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    assert_eq!(
        r.request_refund(
            &id("refund"),
            authorization(&["OVER"]),
            fee(100),
            Timestamp(30)
        )
        .await
        .unwrap(),
        request
    );
    assert_eq!(
        r.refund_status(&id("refund")).await.unwrap(),
        "EXECUTION_GATED"
    );
    assert!(matches!(
        r.request_refund(
            &id("refund"),
            authorization(&["OVER"]),
            fee(200),
            Timestamp(30)
        )
        .await,
        Err(RepositoryError::Domain(Error::IdempotencyConflict))
    ));
    assert!(
        r.request_refund(
            &id("second"),
            authorization(&["OVER"]),
            fee(100),
            Timestamp(30)
        )
        .await
        .is_err()
    );
    assert!(settle(&r, "OVER0001", 31).await.is_err());
    assert_eq!(count(&dir, "refund_reservations").await, 1);
    assert_eq!(count(&dir, "settlement_events").await, 0);
    assert_eq!(count(&dir, "outbound_payouts").await, 0);
}
#[tokio::test]
async fn fee_exhaustion_partial_reservations_and_failed_write_leave_balance_available() {
    let (dir, r) = fixture().await;
    r.insert_quote(&quote("FEE00001", false), Timestamp(1))
        .await
        .unwrap();
    r.confirm(deposit("SMALL1", Some("FEE00001"), 1000), None)
        .await
        .unwrap();
    r.confirm(deposit("SMALL2", Some("FEE00001"), 1000), None)
        .await
        .unwrap();
    for fee_amount in [2000, 2001] {
        assert!(
            r.request_refund(
                &id("bad"),
                authorization(&["SMALL1", "SMALL2"]),
                fee(fee_amount),
                Timestamp(21)
            )
            .await
            .is_err()
        );
    }
    assert!(
        r.request_refund(
            &id("partial"),
            authorization(&["SMALL1"]),
            fee(100),
            Timestamp(21)
        )
        .await
        .is_err()
    );
    let mut c = SqliteConnection::connect(&format!(
        "sqlite://{}",
        dir.path().join("ledger.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER fail_refund BEFORE INSERT ON refund_reservations BEGIN SELECT RAISE(ABORT, 'synthetic fault'); END").execute(&mut c).await.unwrap();
    assert!(
        r.request_refund(
            &id("rollback"),
            authorization(&["SMALL1", "SMALL2"]),
            fee(100),
            Timestamp(21)
        )
        .await
        .is_err()
    );
    assert_eq!(
        r.quote(&id("FEE00001")).await.unwrap().0.state(),
        InboundState::UnderpaidHeld
    );
    assert_eq!(count(&dir, "refund_requests").await, 0);
    sqlx::query("DROP TRIGGER fail_refund")
        .execute(&mut c)
        .await
        .unwrap();
    r.confirm(deposit("REST", Some("FEE00001"), 8000), Some(Timestamp(22)))
        .await
        .unwrap();
    assert_eq!(count(&dir, "settlement_events").await, 1);
}
#[tokio::test]
async fn simultaneous_refund_and_topup_cannot_both_spend_quote_funds() {
    // Both deterministic orderings plus actual separate-pool races.
    for order in 0..4 {
        let (dir, r) = fixture().await;
        r.insert_quote(&quote("RACE0001", false), Timestamp(1))
            .await
            .unwrap();
        r.confirm(deposit("FIRST", Some("RACE0001"), 4000), None)
            .await
            .unwrap();
        let other = Repository::open(dir.path().join("ledger.sqlite"))
            .await
            .unwrap();
        let refund_id = id("refund");
        let refund = r.request_refund(
            &refund_id,
            authorization(&["FIRST"]),
            fee(100),
            Timestamp(22),
        );
        let topup = other.confirm(deposit("TOPUP", Some("RACE0001"), 6000), None);
        let (refund, topup) = match order {
            0 => (refund.await, topup.await),
            1 => {
                let t = topup.await;
                (refund.await, t)
            }
            _ => tokio::join!(refund, topup),
        };
        topup.unwrap();
        if refund.is_ok() {
            assert_eq!(
                r.quote(&id("RACE0001")).await.unwrap().0.state(),
                InboundState::RefundHeld
            );
            assert_eq!(
                r.receipt(&receipt("TOPUP"))
                    .await
                    .unwrap()
                    .unwrap()
                    .allocation,
                Allocation::Exception
            );
            assert!(settle(&r, "RACE0001", 23).await.is_err());
            assert_eq!(count(&dir, "settlement_events").await, 0);
        } else {
            settle(&r, "RACE0001", 23).await.unwrap();
            assert_eq!(count(&dir, "refund_reservations").await, 0);
            assert_eq!(count(&dir, "settlement_events").await, 1);
        }
    }
}
#[tokio::test]
async fn unknown_reference_retains_correlation_and_requires_reviewed_refund_authorization() {
    let (dir, r) = fixture().await;
    r.confirm(deposit("ORPHAN", None, 5000), None)
        .await
        .unwrap();
    let record = r.receipt(&receipt("ORPHAN")).await.unwrap().unwrap();
    assert_eq!(record.state, DepositState::OrphanUnclaimed);
    assert_eq!(
        record.deposit.evidence().rail,
        DepositRail::C2b {
            account_ref: id("TYPO0001")
        }
    );
    assert!(
        r.request_refund(
            &id("missing"),
            authorization(&["UNVERIFIED"]),
            fee(100),
            Timestamp(21)
        )
        .await
        .is_err()
    );
    let request = r
        .request_refund(
            &id("reviewed"),
            authorization(&["ORPHAN"]),
            fee(100),
            Timestamp(21),
        )
        .await
        .unwrap();
    assert_eq!(request.authorization.owner, id("reviewed-owner"));
    assert_eq!(count(&dir, "settlement_events").await, 0);
    assert_eq!(count(&dir, "refund_reservations").await, 1);
}
#[tokio::test]
async fn grace_is_kes_only_receipt_time_bounded_and_never_extends_fx() {
    let (dir, r) = fixture().await;
    for name in ["DEFAULT1", "INGRACE1", "BOUNDARY", "EXPIREDF"] {
        r.insert_quote(&quote(name, name == "EXPIREDF"), Timestamp(1))
            .await
            .unwrap();
    }
    r.confirm(
        VerifiedDeposit::attest(evidence("LATE", Some("DEFAULT1"), 10000, 100, 110)).unwrap(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        r.receipt(&receipt("LATE")).await.unwrap().unwrap().state,
        DepositState::LateHeld
    );
    r.set_deposit_policy(
        &DepositPolicy {
            kes_grace_seconds: 30,
            ..Default::default()
        },
        &id("operator"),
        Timestamp(110),
    )
    .await
    .unwrap();
    r.confirm(
        VerifiedDeposit::attest(evidence("GRACE", Some("INGRACE1"), 10000, 129, 150)).unwrap(),
        Some(Timestamp(150)),
    )
    .await
    .unwrap();
    r.confirm(
        VerifiedDeposit::attest(evidence("EDGE", Some("BOUNDARY"), 10000, 130, 150)).unwrap(),
        None,
    )
    .await
    .unwrap();
    r.confirm(
        VerifiedDeposit::attest(evidence("SAT", Some("EXPIREDF"), 10000, 99, 150)).unwrap(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        r.receipt(&receipt("EDGE")).await.unwrap().unwrap().state,
        DepositState::LateHeld
    );
    assert_eq!(
        r.receipt(&receipt("SAT")).await.unwrap().unwrap().state,
        DepositState::FxExpiredHeld
    );
    assert!(settle(&r, "EXPIREDF", 151).await.is_err());
    assert_eq!(count(&dir, "settlement_events").await, 1);
}
#[tokio::test]
async fn persisted_risk_holds_velocity_release_and_reversal_alerts_preserve_minted_value() {
    let (dir, r) = fixture().await;
    r.set_deposit_policy(
        &DepositPolicy {
            high_value_minor: 10000,
            hold_seconds: 30,
            velocity_window_seconds: 100,
            velocity_limit: 1,
            ..Default::default()
        },
        &id("operator"),
        Timestamp(1),
    )
    .await
    .unwrap();
    for name in ["RISK0001", "RISK0002", "TRUSTED1"] {
        r.insert_quote(&quote(name, false), Timestamp(1))
            .await
            .unwrap();
    }
    r.confirm(deposit("HIGH1", Some("RISK0001"), 10000), None)
        .await
        .unwrap();
    assert!(matches!(
        settle(&r, "RISK0001", 49).await,
        Err(RepositoryError::Domain(Error::SettlementHeld))
    ));
    r.close().await;
    let r = Repository::open(dir.path().join("ledger.sqlite"))
        .await
        .unwrap();
    settle(&r, "RISK0001", 50).await.unwrap();
    r.confirm(deposit("HIGH2", Some("RISK0002"), 10000), None)
        .await
        .unwrap();
    assert!(settle(&r, "RISK0002", 51).await.is_err());
    use daraja_core::{
        crypto::Secret,
        operations::{OperatorCredential, OperatorRole, OperatorService, ReviewDepositHold},
    };
    let token = Secret::new("synthetic-review-token-000000000000000000");
    let service = OperatorService::new(
        r.clone(),
        vec![OperatorCredential::new(id("reviewer"), &token, OperatorRole::Reconciler).unwrap()],
    )
    .unwrap();
    let request = ReviewDepositHold {
        command_id: id("review-1"),
        quote_id: id("RISK0002"),
        expected_reason: id("VELOCITY"),
        reason: id("case-1"),
        evidence_ref: id("statement-1"),
    };
    assert!(
        service
            .review_deposit_hold(&Secret::new("unauthorized"), &request, Timestamp(51))
            .await
            .is_err()
    );
    let mut stale = request.clone();
    stale.expected_reason = id("REVERSAL_REPORTED");
    assert!(
        service
            .review_deposit_hold(&token, &stale, Timestamp(51))
            .await
            .is_err()
    );
    let (a, b) = tokio::join!(
        service.review_deposit_hold(&token, &request, Timestamp(51)),
        service.review_deposit_hold(&token, &request, Timestamp(51))
    );
    a.unwrap();
    b.unwrap();
    let mut conflict = request.clone();
    conflict.evidence_ref = id("other-evidence");
    assert!(
        service
            .review_deposit_hold(&token, &conflict, Timestamp(52))
            .await
            .is_err()
    );
    settle(&r, "RISK0002", 51).await.unwrap();
    let mut e = evidence("KNOWN", Some("TRUSTED1"), 10000, 10, 20);
    e.payer = Some(VerifiedPayer {
        subject: id("verified-pseudonym"),
        identity_verified: true,
    });
    r.confirm(VerifiedDeposit::attest(e).unwrap(), Some(Timestamp(21)))
        .await
        .unwrap();
    r.report_reversal(&receipt("HIGH1"), &id("synthetic-reversal"), Timestamp(60))
        .await
        .unwrap();
    r.report_reversal(&receipt("HIGH1"), &id("synthetic-reversal"), Timestamp(61))
        .await
        .unwrap();
    assert_eq!(
        r.quote(&id("RISK0001")).await.unwrap().0.state(),
        InboundState::Paid
    );
    assert_eq!(count(&dir, "settlement_events").await, 3);
    assert_eq!(count(&dir, "deposit_alerts").await, 2);
    assert!(
        r.request_refund(
            &id("paid-refund"),
            authorization(&["HIGH1"]),
            fee(100),
            Timestamp(61)
        )
        .await
        .is_err()
    );
    r.confirm(deposit("EXTRA", Some("RISK0001"), 2000), None)
        .await
        .unwrap();
    r.request_refund(
        &id("extra-refund"),
        authorization(&["EXTRA"]),
        fee(100),
        Timestamp(61),
    )
    .await
    .unwrap();
    assert_eq!(count(&dir, "settlement_events").await, 3);
}

#[tokio::test]
async fn legacy_verified_receipts_decode_without_payer_and_new_policy_counts_duplicates_once() {
    let (dir, r) = fixture().await;
    r.insert_quote(&quote("LEGACY01", false), Timestamp(1))
        .await
        .unwrap();
    r.confirm(deposit("OLD", Some("LEGACY01"), 4000), None)
        .await
        .unwrap();
    let mut c = SqliteConnection::connect(&format!(
        "sqlite://{}",
        dir.path().join("ledger.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE mpesa_receipts_ledger SET data=json_remove(data,'$.deposit.payer') WHERE receipt='OLD'").execute(&mut c).await.unwrap();
    assert_eq!(
        r.receipt(&receipt("OLD"))
            .await
            .unwrap()
            .unwrap()
            .deposit
            .evidence()
            .payer,
        None
    );
    r.set_deposit_policy(
        &DepositPolicy {
            high_value_minor: 1000,
            hold_seconds: 30,
            velocity_window_seconds: 100,
            velocity_limit: 1,
            ..Default::default()
        },
        &id("operator"),
        Timestamp(21),
    )
    .await
    .unwrap();
    r.confirm(deposit("NEW", Some("LEGACY01"), 6000), None)
        .await
        .unwrap();
    assert!(
        r.confirm(deposit("NEW", Some("LEGACY01"), 6000), None)
            .await
            .is_err()
    );
    assert_eq!(count(&dir, "deposit_risk").await, 1);
    assert_eq!(count(&dir, "deposit_blocks").await, 0);
    assert!(settle(&r, "LEGACY01", 49).await.is_err());
    settle(&r, "LEGACY01", 50).await.unwrap();
}

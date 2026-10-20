#[path = "support/liquidity_fixture.rs"]
mod liquidity_fixture;
use daraja_core::{
    Error, Timestamp,
    amount::{Amount, FxSnapshot, Unit},
    fx::*,
    identity::{Id, ReceiptId, RequestKey, RequestPayload},
    inbound::EvidenceSource,
    outbound::*,
    repository::{PayoutChange, Repository},
    tariffs::*,
    valuation::PaymentQuote,
};
use std::{collections::BTreeMap, time::Duration};
fn id(s: &str) -> Id {
    Id::new(s).unwrap()
}
fn kes(v: u64) -> Amount {
    Amount::new(v, Unit::KesMinor)
}
fn sat(v: u64) -> Amount {
    Amount::new(v, Unit::Sat)
}
fn fx() -> FxSnapshot {
    FxSnapshot {
        source: id("synthetic"),
        numerator: 7,
        denominator: 13,
        expires_at: Timestamp(190),
    }
}
fn schedule(name: &str, fee: u64) -> Schedule {
    Schedule {
        version: id(name),
        effective_from: Timestamp(100),
        effective_until: Timestamp(200),
        bands: vec![
            Band {
                min_kes: 1,
                max_kes: 10,
                fee: kes(fee),
            },
            Band {
                min_kes: 11,
                max_kes: 1000,
                fee: kes(fee + 7),
            },
        ],
    }
}
fn tariffs() -> Tariffs {
    Tariffs::new(
        vec![schedule("synthetic-b2c", 3)],
        vec![schedule("synthetic-b2b", 11)],
    )
    .unwrap()
}
fn quote(unit: Unit) -> PaymentQuote {
    PaymentQuote::new(
        id("quote"),
        "0712345678",
        kes(1100),
        unit,
        &tariffs(),
        if unit == Unit::Sat { Some(fx()) } else { None },
        Timestamp(110),
        Timestamp(199),
    )
    .unwrap()
}

#[test]
fn inclusive_amount_and_exclusive_version_boundaries() {
    let t = tariffs();
    for (principal, fee) in [(100, 3), (1000, 3), (1100, 10), (100000, 10)] {
        assert_eq!(
            t.quote(Rail::B2c, kes(principal), Timestamp(100))
                .unwrap()
                .fee,
            kes(fee)
        );
    }
    assert_eq!(
        t.quote(Rail::B2b, kes(1000), Timestamp(199)).unwrap().fee,
        kes(11)
    );
    for n in [0, 1, 99, 100001, 100100] {
        assert!(t.quote(Rail::B2c, kes(n), Timestamp(110)).is_err());
    }
    for time in [99, 200] {
        assert!(t.quote(Rail::B2c, kes(100), Timestamp(time)).is_err());
    }
    let mut next = schedule("new", 17);
    next.effective_from = Timestamp(200);
    next.effective_until = Timestamp(300);
    let t = Tariffs::new(vec![schedule("old", 3), next], vec![schedule("b2b", 8)]).unwrap();
    assert_eq!(
        t.quote(Rail::B2c, kes(100), Timestamp(200)).unwrap().fee,
        kes(17)
    );
}
#[test]
fn reject_gaps_overlaps_negative_decimal_and_overflow() {
    for start in [9, 10, 12] {
        let mut s = schedule("bad", 0);
        s.bands[1].min_kes = start;
        assert!(Tariffs::new(vec![s], vec![schedule("b", 0)]).is_err());
    }
    for time in [199, 201] {
        let mut s = schedule("next", 0);
        s.effective_from = Timestamp(time);
        s.effective_until = Timestamp(300);
        assert!(Tariffs::new(vec![schedule("a", 0), s], vec![schedule("b", 0)]).is_err());
    }
    for bad in ["-1", "+1", "1.001", "1e2", "NaN", "", ".5", "1."] {
        assert!(Amount::kes_decimal(bad).is_err());
    }
    assert_eq!(Amount::kes_decimal("1.20").unwrap(), kes(120));
    assert_eq!(Amount::kes_decimal("0").unwrap(), kes(0));
    assert!(Amount::kes_decimal("18446744073709551615").is_err());
    let mut s = schedule("bad", 0);
    s.bands[0].fee = sat(1);
    assert!(Tariffs::new(vec![s], vec![schedule("b", 0)]).is_err());
    let mut s = schedule("bad", 0);
    s.bands[0].fee = kes(u64::MAX);
    let t = Tariffs::new(vec![s], vec![schedule("b", 0)]).unwrap();
    assert_eq!(
        PaymentQuote::new(
            id("q"),
            "0712345678",
            kes(100),
            Unit::KesMinor,
            &t,
            None,
            Timestamp(110),
            Timestamp(150)
        ),
        Err(Error::Overflow)
    );
}
#[test]
fn rounding_and_fees_never_create_value_or_lose_float() {
    let f = fx();
    for v in 1..20000 {
        let deposit = kes(v);
        let issued = f.credit(deposit).unwrap();
        assert!(f.payout_kes(issued).unwrap().value() <= v);
        let backing = f.backing_kes(sat(v)).unwrap();
        assert!(f.credit(backing).unwrap().value() >= v);
        assert!(f.debit(f.payout_kes(sat(v)).unwrap()).unwrap().value() <= v);
    }
    let q = quote(Unit::Sat);
    assert_eq!(
        q.fee_reserve()
            .unwrap()
            .checked_add(q.wallet_principal())
            .unwrap(),
        q.wallet_reserved()
    );
    assert_eq!(q.total_spent(None).unwrap(), q.wallet_reserved());
    assert_eq!(q.total_spent(Some(kes(11))), Err(Error::SettlementHeld));
    for fee in 0..=10 {
        let spent = q.total_spent(Some(kes(fee))).unwrap();
        assert!(spent.value() <= q.wallet_reserved().value());
        assert!(u128::from(spent.value()) * 13 >= u128::from(1100 + fee) * 7);
    }
    let mut huge = f;
    huge.numerator = u64::MAX;
    huge.denominator = 1;
    assert_eq!(huge.credit(kes(u64::MAX)), Err(Error::Overflow));
    assert!(
        quote(Unit::KesMinor)
            .validate_dispatch(Timestamp(199))
            .is_err()
    );
    assert!(q.validate_dispatch(Timestamp(190)).is_err());
    assert!(
        PaymentQuote::new(
            id("q"),
            "TILL:123456",
            kes(0),
            Unit::Sat,
            &tariffs(),
            Some(fx()),
            Timestamp(110),
            Timestamp(180)
        )
        .is_err()
    );
    assert_eq!(fx().payout_kes(sat(1)).unwrap(), kes(0));
}
#[tokio::test]
async fn oracle_errors_malformed_and_stale_are_unavailable() {
    assert_eq!(
        OfflineOracle { snapshot: None }.quote(Timestamp(110)).await,
        Err(FxError::Unavailable)
    );
    assert!(
        OfflineOracle {
            snapshot: Some(fx())
        }
        .quote(Timestamp(190))
        .await
        .is_err()
    );
    for bad in [
        "0",
        "-1",
        "NaN",
        "1e3",
        "1.123456789",
        "18446744073709551616",
    ] {
        let body = format!(r#"{{"data":{{"currency":"BTC","rates":{{"KES":"{bad}"}}}}}}"#);
        assert!(CoinbaseOracle::parse(body.as_bytes(), Timestamp(110), Timestamp(180)).is_err());
    }
    for bad in [
        r#"{"data":{"currency":"USD","rates":{"KES":"130"}}}"#,
        r#"{"data":{"currency":"BTC","rates":{}}}"#,
        r#"{"data":{"currency":"BTC","rates":{"KES":null}}}"#,
        r#"{"data":{"currency":"BTC","rates":{"KES":130}}}"#,
        r#"{"data":{"currency":"BTC","rates":{"KES":"1","KES":"2"}}}"#,
    ] {
        assert!(CoinbaseOracle::parse(bad.as_bytes(), Timestamp(110), Timestamp(180)).is_err());
    }
    let f = CoinbaseOracle::parse(
        br#"{"data":{"currency":"BTC","rates":{"KES":"12345678.12"}}}"#,
        Timestamp(110),
        Timestamp(180),
    )
    .unwrap();
    assert_eq!(f.numerator, 100_000_000);
    assert_eq!(f.denominator, 1_234_567_812);
}

#[tokio::test]
async fn concrete_oracle_http_contract_age_and_failure_boundary() {
    use axum::{Router, extract::Query, http::StatusCode, response::IntoResponse, routing::get};
    let app = Router::new()
        .route(
            "/v2/exchange-rates",
            get(|Query(q): Query<BTreeMap<String, String>>| async move {
                assert_eq!(q.get("currency").unwrap(), "BTC");
                (
                    [("age", "2")],
                    r#"{"data":{"currency":"BTC","rates":{"KES":"12345678.12"}}}"#,
                )
            }),
        )
        .route("/stale", get(|| async { ([("age", "30")], "{}") }))
        .route(
            "/error",
            get(|| async { StatusCode::SERVICE_UNAVAILABLE.into_response() }),
        )
        .route("/large", get(|| async { "x".repeat(65537) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let make = |path| {
        CoinbaseOracle::new(
            &format!("http://{addr}{path}"),
            20,
            30,
            Duration::from_secs(1),
        )
        .unwrap()
    };
    assert_eq!(
        make("/v2/exchange-rates")
            .quote(Timestamp(110))
            .await
            .unwrap()
            .expires_at,
        Timestamp(130)
    );
    assert_eq!(
        make("/stale").quote(Timestamp(110)).await,
        Err(FxError::Stale)
    );
    assert_eq!(
        make("/error").quote(Timestamp(110)).await,
        Err(FxError::Unavailable)
    );
    assert_eq!(
        make("/large").quote(Timestamp(110)).await,
        Err(FxError::Malformed)
    );
    task.abort();
}

#[tokio::test]
async fn frozen_quote_restart_dispatch_expiry_and_actual_fee_audit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let repo = Repository::open(&path).await.unwrap();
    liquidity_fixture::fund(&repo).await;
    let q = quote(Unit::Sat);
    repo.save_payment_quote(&q, Timestamp(110)).await.unwrap();
    drop(repo);
    let repo = Repository::open(&path).await.unwrap();
    assert_eq!(repo.payment_quote(q.id()).await.unwrap(), q);
    let changed = PaymentQuote::new(
        q.id().clone(),
        "0712345678",
        kes(1100),
        Unit::Sat,
        &Tariffs::new(vec![schedule("reload", 90)], vec![schedule("b", 0)]).unwrap(),
        Some(fx()),
        Timestamp(110),
        Timestamp(180),
    )
    .unwrap();
    assert!(
        repo.save_payment_quote(&changed, Timestamp(110))
            .await
            .is_err()
    );
    repo.save_payment_quote(&q, Timestamp(300)).await.unwrap();
    assert!(q.validate_dispatch(Timestamp(300)).is_err());
    let payload = RequestPayload {
        operation: id("outbound"),
        quote_id: q.id().clone(),
        destination: q.destination().clone(),
        amount: q.principal(),
        tariff_version: q.tariff().version.clone(),
        options: BTreeMap::new(),
    };
    let d = DispatchRecord::new(
        id("operation"),
        payload,
        br#"{"PartyA":"600000"}"#.to_vec(),
        q.tariff().clone(),
        q.principal().checked_add(q.tariff().fee).unwrap(),
    )
    .unwrap();
    assert!(d.clone().with_quote(q.clone(), Timestamp(190)).is_err());
    let d = d.with_quote(q.clone(), Timestamp(120)).unwrap();
    assert!(
        Payout::dispatching(
            d.clone(),
            id("caller"),
            Timestamp(190),
            Timestamp(200),
            Timestamp(300)
        )
        .is_err()
    );
    let p = Payout::dispatching(
        d,
        id("caller"),
        Timestamp(120),
        Timestamp(140),
        Timestamp(300),
    )
    .unwrap();
    let key = RequestKey {
        caller: id("caller"),
        operation: id("outbound"),
        key: id("key"),
    };
    repo.prepare_payout(&key, &p).await.unwrap();
    assert!(
        repo.record_payout_fee(&id("operation"), Some(kes(5)), Timestamp(130))
            .await
            .is_err()
    );
    let (_, version) = repo.payout(&id("operation")).await.unwrap();
    repo.change_payout(
        &id("operation"),
        version,
        PayoutChange::Resolve(PayoutAttestation {
            source: EvidenceSource::AuthenticatedProviderRecord(id("synthetic")),
            operation_id: id("operation"),
            payload_hash: p.dispatch().hash(),
            amount: q.principal(),
            destination: q.destination().clone(),
            outcome: FinalOutcome::Paid {
                receipt: ReceiptId::new("SYNTH123").unwrap(),
            },
        }),
        Timestamp(130),
    )
    .await
    .unwrap();
    assert_eq!(
        repo.record_payout_fee(&id("operation"), Some(kes(5)), Timestamp(130))
            .await
            .unwrap(),
        q.total_spent(Some(kes(5))).unwrap()
    );
    assert!(
        repo.record_payout_fee(&id("operation"), Some(kes(6)), Timestamp(131))
            .await
            .is_err()
    );
    drop(repo);
    let repo = Repository::open(&path).await.unwrap();
    assert_eq!(
        repo.record_payout_fee(&id("operation"), Some(kes(5)), Timestamp(301))
            .await
            .unwrap(),
        q.total_spent(Some(kes(5))).unwrap()
    );
    for (name, actual) in [("unknown", None), ("excess", Some(kes(20)))] {
        let q2 = PaymentQuote::new(
            id(name),
            "0712345678",
            kes(1100),
            Unit::Sat,
            &tariffs(),
            Some(fx()),
            Timestamp(110),
            Timestamp(180),
        )
        .unwrap();
        repo.save_payment_quote(&q2, Timestamp(110)).await.unwrap();
        let payload = RequestPayload {
            operation: id("outbound"),
            quote_id: q2.id().clone(),
            destination: q2.destination().clone(),
            amount: q2.principal(),
            tariff_version: q2.tariff().version.clone(),
            options: BTreeMap::new(),
        };
        let d = DispatchRecord::new(
            id(name),
            payload,
            br#"{"PartyA":"600000"}"#.to_vec(),
            q2.tariff().clone(),
            q2.principal().checked_add(q2.tariff().fee).unwrap(),
        )
        .unwrap()
        .with_quote(q2.clone(), Timestamp(120))
        .unwrap();
        let p = Payout::dispatching(d, id(name), Timestamp(120), Timestamp(140), Timestamp(300))
            .unwrap();
        repo.prepare_payout(
            &RequestKey {
                caller: id("caller"),
                operation: id("outbound"),
                key: id(name),
            },
            &p,
        )
        .await
        .unwrap();
        let (_, v) = repo.payout(&id(name)).await.unwrap();
        repo.change_payout(
            &id(name),
            v,
            PayoutChange::Resolve(PayoutAttestation {
                source: EvidenceSource::ReviewedStatement(id("synthetic")),
                operation_id: id(name),
                payload_hash: p.dispatch().hash(),
                amount: q2.principal(),
                destination: q2.destination().clone(),
                outcome: FinalOutcome::Paid {
                    receipt: ReceiptId::new(name).unwrap(),
                },
            }),
            Timestamp(130),
        )
        .await
        .unwrap();
        let result = repo
            .record_payout_fee(&id(name), actual, Timestamp(300))
            .await;
        if actual.is_none() {
            assert_eq!(result.unwrap(), q2.wallet_reserved());
        } else {
            assert!(result.is_err());
        }
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&path))
            .await
            .unwrap();
        let fee: Option<String> =
            sqlx::query_scalar("SELECT actual_fee FROM payout_fee_audit WHERE payout_id=?")
                .bind(name)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(fee, actual.map(|a| serde_json::to_string(&a).unwrap()));
        let total: Option<String> =
            sqlx::query_scalar("SELECT total_spent FROM payout_fee_audit WHERE payout_id=?")
                .bind(name)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(total.is_none(), actual.is_some());
        assert!(
            sqlx::query("UPDATE payment_quotes SET data=data WHERE id=?")
                .bind(name)
                .execute(&pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM payout_fee_audit WHERE payout_id=?")
                .bind(name)
                .execute(&pool)
                .await
                .is_err()
        );
    }
}

#[test]
fn sat_budget_includes_tariff_and_enforces_minimum_payout() {
    assert!(
        PaymentQuote::from_sat_budget(
            id("tiny"),
            "0712345678",
            sat(1),
            &tariffs(),
            fx(),
            Timestamp(110),
            Timestamp(180)
        )
        .is_err()
    );
    for budget in 100..10000 {
        let q = PaymentQuote::from_sat_budget(
            id("budget"),
            "TILL:123456",
            sat(budget),
            &tariffs(),
            fx(),
            Timestamp(110),
            Timestamp(180),
        )
        .unwrap();
        assert!(q.wallet_reserved().value() <= budget);
        let total = q.principal().checked_add(q.tariff().fee).unwrap().value();
        assert!(u128::from(q.wallet_reserved().value()) * 13 >= u128::from(total) * 7);
    }
    let q = quote(Unit::KesMinor);
    assert_eq!(q.cdk_principal().unwrap(), 11);
    assert_eq!(q.cdk_fee_reserve().unwrap(), 1);
    assert_eq!(q.cdk_total_spent(Some(kes(1))).unwrap(), 12);
    assert_eq!(q.cdk_total_spent(Some(kes(0))).unwrap(), 11);
}

use daraja_core::{
    Timestamp,
    amount::{Amount, TariffSnapshot, Unit},
    crypto::Secret,
    identity::{Id, ReceiptId},
    inbound::{
        DepositAttestation, DepositRail, EvidenceSource, InboundQuote, InboundState, QuoteTerms,
        StkAttempt, VerifiedDeposit,
    },
    repository::{Repository, StkChange},
    webhook::{Endpoint, Policy, router},
};
use serde_json::{Value, json};
use sqlx::{Connection, SqliteConnection};
use std::{
    collections::BTreeMap,
    io::Write,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn id(s: &str) -> Id {
    Id::new(s).unwrap()
}
fn kes(n: u64) -> Amount {
    Amount::new(n, Unit::KesMinor)
}
fn secret(endpoint: Endpoint) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(format!("synthetic-only-{endpoint:?}"))
    )
}
fn policy(enabled: bool, topups: bool) -> Policy {
    let secrets = Endpoint::ALL
        .into_iter()
        .map(|e| (e, Secret::new(secret(e))))
        .collect::<BTreeMap<_, _>>();
    let p = Policy::new(id("12345"), secrets).unwrap();
    if enabled {
        p.enable_verified_c2b_validation(topups)
    } else {
        p
    }
}
fn c2b(account: &str, amount: &str) -> Value {
    json!({"TransactionType":"Pay Bill","TransID":"SYNTH123", "TransTime":"20261020123456",
        "TransAmount":amount,"BusinessShortCode":"12345", "BillRefNumber":account,"MSISDN":"254712345678"})
}
fn stk(checkout: &str, amount: u64) -> Value {
    json!({"Body":{"stkCallback":{"MerchantRequestID":"merchant", "CheckoutRequestID":checkout,
        "ResultCode":0,"ResultDesc":"Success","CallbackMetadata":{"Item":[
            {"Name":"Amount","Value":amount}, {"Name":"MpesaReceiptNumber","Value":"SYNTH123"},
            {"Name":"TransactionDate","Value":20261020123456_u64}, {"Name":"PhoneNumber","Value":254712345678_u64}]}}}})
}
fn result() -> Value {
    json!({"Result":{"ResultCode":0,"ResultDesc":"Success","OriginatorConversationID":"originator",
        "ConversationID":"conversation", "TransactionID":"SYNTH456"}})
}
async fn serve(repo: Repository, policy: Policy) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router(repo, policy).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (address, server)
}
async fn post(
    address: SocketAddr,
    endpoint: Endpoint,
    key: &str,
    body: &str,
    headers: &str,
) -> (u16, Value) {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let path = endpoint.path().replace("{secret}", key);
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        serde_json::from_str(body).unwrap(),
    )
}
async fn send(address: SocketAddr, endpoint: Endpoint, body: Value) -> (u16, Value) {
    post(address, endpoint, &secret(endpoint), &body.to_string(), "").await
}
async fn count(db: &mut SqliteConnection, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(db)
        .await
        .unwrap()
}
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Self;
    fn make_writer(&'a self) -> Self {
        self.clone()
    }
}

#[tokio::test]
async fn real_routes_enforce_admission_durability_validation_and_redaction() {
    let capture = Capture::default();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .without_time()
            .with_writer(capture.clone())
            .finish(),
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite");
    let repo = Repository::open(&path).await.unwrap();
    let now = Timestamp(chrono::Utc::now().timestamp() as u64);
    let quote = InboundQuote::new(
        id("quote"),
        id("CDK9F2A1"),
        id("12345"),
        QuoteTerms {
            expected: kes(10000),
            credit: kes(9900),
            quote_expires_at: Timestamp(now.0 + 3600),
            fx: None,
            tariff: TariffSnapshot {
                version: id("v1"),
                fee: kes(100),
            },
            hold_until: Timestamp(0),
        },
    )
    .unwrap();
    repo.insert_quote(&quote, now).await.unwrap();
    repo.change_stk(
        quote.id(),
        0,
        StkChange::Start(StkAttempt::new(id("attempt"), id("caller"), now).unwrap()),
        now,
    )
    .await
    .unwrap();
    repo.change_stk(
        quote.id(),
        1,
        StkChange::Acknowledge {
            attempt: id("attempt"),
            merchant: id("merchant"),
            checkout: id("checkout"),
        },
        now,
    )
    .await
    .unwrap();
    let mut db = SqliteConnection::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    let (address, server) = serve(repo.clone(), policy(true, true)).await;
    for endpoint in Endpoint::ALL {
        let response = post(address, endpoint, &"a".repeat(64), "{broken", "").await;
        assert_eq!(response.0, 404, "{endpoint:?}");
        assert_eq!(
            post(address, endpoint, &secret(endpoint), "{broken", "")
                .await
                .0,
            400
        );
        assert_eq!(
            post(address, endpoint, &secret(endpoint), &"x".repeat(32769), "")
                .await
                .0,
            413
        );
        assert_eq!(
            post(
                address,
                endpoint,
                &secret(endpoint),
                "{}",
                "Forwarded: for=196.201.212.1\r\n"
            )
            .await
            .0,
            403
        );
        assert_eq!(
            post(
                address,
                endpoint,
                &secret(endpoint),
                "{}",
                "X-Forwarded-For: 196.201.212.1\r\n"
            )
            .await
            .0,
            403
        );
    }
    for endpoint in Endpoint::ALL {
        assert_eq!(
            send(address, endpoint, json!({})).await.0,
            400,
            "{endpoint:?}"
        );
    }
    assert_eq!(count(&mut db, "callback_inbox").await, 0);
    let validate = Endpoint::C2bValidation;
    assert_eq!(
        send(address, validate, c2b("TYPO0001", "100")).await.1["ResultCode"],
        "C2B00012"
    );
    for amount in ["99", "101", "NaN", "-1", "0"] {
        assert_eq!(
            send(address, validate, c2b("CDK9F2A1", amount)).await.1["ResultCode"],
            "C2B00013"
        );
    }
    assert_eq!(
        send(address, validate, c2b("CDK9F2A1", "100")).await.1["ResultCode"],
        "0"
    );
    let mut wrong = c2b("CDK9F2A1", "100");
    wrong["BusinessShortCode"] = json!("99999");
    assert_eq!(
        send(address, validate, wrong.clone()).await.1["ResultCode"],
        "C2B00012"
    );
    assert_eq!(send(address, Endpoint::C2bConfirmation, wrong).await.0, 400);
    assert_eq!(
        send(address, Endpoint::Stk, stk("unknown", 100)).await.0,
        400
    );
    let mut wrong = stk("checkout", 100);
    wrong["Body"]["stkCallback"]["MerchantRequestID"] = json!("wrong");
    assert_eq!(send(address, Endpoint::Stk, wrong).await.0, 400);
    assert_eq!(
        send(address, Endpoint::Stk, stk("checkout", 100)).await.0,
        200
    ); // No BusinessShortCode in STK.
    let failure = json!({"Body":{"stkCallback":{"MerchantRequestID":"merchant", "CheckoutRequestID":"checkout",
        "ResultCode":1032,"ResultDesc":"Cancelled"}}});
    assert_eq!(send(address, Endpoint::Stk, failure.clone()).await.0, 200);
    assert_eq!(send(address, Endpoint::Stk, failure).await.0, 200);
    let mut missing_metadata = stk("checkout", 100);
    missing_metadata["Body"]["stkCallback"]["CallbackMetadata"] = Value::Null;
    assert_eq!(send(address, Endpoint::Stk, missing_metadata).await.0, 400);
    let (one, two) = tokio::join!(
        send(address, Endpoint::C2bConfirmation, c2b("CDK9F2A1", "100")),
        send(address, Endpoint::C2bConfirmation, c2b("CDK9F2A1", "100"))
    );
    assert_eq!((one.0, two.0), (200, 200));
    assert_eq!(count(&mut db, "callback_inbox").await, 3);
    assert_eq!(count(&mut db, "callback_jobs").await, 3);
    // Amount mismatch and orphan receipts must survive for independently verified exception handling.
    assert_eq!(
        send(address, Endpoint::Stk, stk("checkout", 90)).await.0,
        200
    );
    assert_eq!(
        send(address, Endpoint::C2bConfirmation, c2b("TYPO0001", "101"))
            .await
            .0,
        200
    );
    for endpoint in [
        Endpoint::B2c,
        Endpoint::B2b,
        Endpoint::B2cTimeout,
        Endpoint::B2bTimeout,
        Endpoint::StatusResult,
        Endpoint::StatusTimeout,
        Endpoint::BalanceResult,
        Endpoint::BalanceTimeout,
    ] {
        assert_eq!(
            send(address, endpoint, result()).await.0,
            200,
            "{endpoint:?}"
        );
        assert_eq!(send(address, endpoint, result()).await.0, 200);
    }
    assert_eq!(count(&mut db, "callback_jobs").await, 13);
    assert_eq!(count(&mut db, "mpesa_receipts_ledger").await, 0);
    assert_eq!(count(&mut db, "settlement_events").await, 0);
    assert_eq!(
        repo.quote(&id("quote")).await.unwrap().0.state(),
        InboundState::StkSent
    );
    let duplicate_key = r#"{"TransID":"SYNTH123","TransID":"SPOOF"}"#;
    assert_eq!(
        post(
            address,
            Endpoint::C2bConfirmation,
            &secret(Endpoint::C2bConfirmation),
            duplicate_key,
            ""
        )
        .await
        .0,
        400
    );
    // Inject a failure between observation and job writes: no acknowledgment and no partial observation.
    sqlx::query("CREATE TRIGGER fail_callback_job BEFORE INSERT ON callback_jobs BEGIN SELECT RAISE(ABORT, 'synthetic fault'); END").execute(&mut db).await.unwrap();
    assert_eq!(
        send(address, Endpoint::C2bConfirmation, c2b("CDK9F2A1", "50"))
            .await
            .0,
        503
    );
    assert_eq!(count(&mut db, "callback_inbox").await, 13);
    sqlx::query("DROP TRIGGER fail_callback_job")
        .execute(&mut db)
        .await
        .unwrap();
    assert_eq!(
        send(address, Endpoint::C2bConfirmation, c2b("CDK9F2A1", "50"))
            .await
            .0,
        200
    );
    // A forged same-receipt observation cannot consume the verified allocation slot.
    repo.confirm(
        VerifiedDeposit::attest(DepositAttestation {
            source: EvidenceSource::ReviewedStatement(id("synthetic-independent-evidence")),
            receipt: ReceiptId::new("SYNTH123").unwrap(),
            quote_id: Some(id("quote")),
            amount: kes(4000),
            shortcode: id("12345"),
            rail: DepositRail::C2b {
                account_ref: id("CDK9F2A1"),
            },
            received_at: now,
            verified_at: now,
        })
        .unwrap(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        send(address, validate, c2b("CDK9F2A1", "60")).await.1["ResultCode"],
        "0"
    );
    for amount in ["100", "40", "59", "61"] {
        assert_eq!(
            send(address, validate, c2b("CDK9F2A1", amount)).await.1["ResultCode"],
            "C2B00013"
        );
    }
    assert_eq!(count(&mut db, "mpesa_receipts_ledger").await, 1);
    let (off_address, off_server) = serve(repo.clone(), policy(false, false)).await;
    assert_eq!(
        send(off_address, validate, c2b("CDK9F2A1", "60")).await.0,
        503
    );
    off_server.abort();
    let (no_topup_address, no_topup_server) = serve(repo.clone(), policy(true, false)).await;
    assert_eq!(
        send(no_topup_address, validate, c2b("CDK9F2A1", "60"))
            .await
            .1["ResultCode"],
        "C2B00012"
    );
    no_topup_server.abort();
    let (filtered_address, filtered_server) = serve(
        repo.clone(),
        policy(true, true).restrict_direct_peers(vec!["192.0.2.0/24".parse().unwrap()]),
    )
    .await;
    assert_eq!(
        send(filtered_address, validate, c2b("CDK9F2A1", "60"))
            .await
            .0,
        403
    );
    assert_eq!(
        post(
            filtered_address,
            validate,
            &secret(validate),
            "{}",
            "X-Real-IP: 192.0.2.1\r\n"
        )
        .await
        .0,
        403
    );
    filtered_server.abort();
    // A slow sender is bounded by the same deadline as local validation/storage.
    let mut slow = tokio::net::TcpStream::connect(address).await.unwrap();
    slow.write_all(format!("POST {} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{{", validate.path().replace("{secret}", &secret(validate))).as_bytes()).await.unwrap();
    let mut response = [0; 1024];
    let read = tokio::time::timeout(Duration::from_secs(2), slow.read(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&response[..read]).starts_with("HTTP/1.1 503"));
    repo.close().await;
    assert_eq!(
        send(address, Endpoint::C2bConfirmation, c2b("CDK9F2A1", "70"))
            .await
            .0,
        503
    );
    server.abort();
    let reopened = Repository::open(&path).await.unwrap();
    assert_eq!(count(&mut db, "callback_inbox").await, 14);
    assert_eq!(count(&mut db, "callback_jobs").await, 14);
    let (address, server) = serve(reopened.clone(), policy(true, true)).await;
    assert_eq!(
        send(address, Endpoint::C2bConfirmation, c2b("CDK9F2A1", "50"))
            .await
            .0,
        200
    );
    assert_eq!(count(&mut db, "callback_jobs").await, 14);
    server.abort();
    reopened.close().await;
    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(!logs.is_empty(), "TRACE capture must be active");
    assert!(!logs.contains("254712345678"), "MSISDN leaked");
    for endpoint in Endpoint::ALL {
        assert!(!logs.contains(&secret(endpoint)), "URL secret leaked");
    }
}

#[tokio::test]
async fn validation_rejects_expired_terms_and_nonactive_quotes() {
    use daraja_core::amount::FxSnapshot;
    let directory = tempfile::tempdir().unwrap();
    let repo = Repository::open(directory.path().join("expiry.sqlite"))
        .await
        .unwrap();
    let now = Timestamp(chrono::Utc::now().timestamp() as u64);
    let terms = QuoteTerms {
        expected: kes(10000),
        credit: kes(9900),
        quote_expires_at: Timestamp(now.0 + 3600),
        fx: None,
        tariff: TariffSnapshot {
            version: id("v1"),
            fee: kes(100),
        },
        hold_until: Timestamp(0),
    };
    for (account, shortcode, quote_terms) in [
        ("ACTIVE01", "12345", terms.clone()),
        (
            "EXPIRED1",
            "12345",
            QuoteTerms {
                quote_expires_at: now,
                ..terms.clone()
            },
        ),
        ("WRONG001", "99999", terms.clone()),
        (
            "STALEFX1",
            "12345",
            QuoteTerms {
                credit: Amount::new(9900, Unit::Sat),
                fx: Some(FxSnapshot {
                    source: id("synthetic-fx"),
                    numerator: 1,
                    denominator: 1,
                    expires_at: now,
                }),
                ..terms.clone()
            },
        ),
        ("VERIFIED", "12345", terms),
    ] {
        repo.insert_quote(
            &InboundQuote::new(id(account), id(account), id(shortcode), quote_terms).unwrap(),
            now,
        )
        .await
        .unwrap();
    }
    repo.confirm(
        VerifiedDeposit::attest(DepositAttestation {
            source: EvidenceSource::ReviewedStatement(id("synthetic-only")),
            receipt: ReceiptId::new("FULLVALUE").unwrap(),
            quote_id: Some(id("VERIFIED")),
            amount: kes(10000),
            shortcode: id("12345"),
            rail: DepositRail::C2b {
                account_ref: id("VERIFIED"),
            },
            received_at: now,
            verified_at: now,
        })
        .unwrap(),
        None,
    )
    .await
    .unwrap();
    let (address, server) = serve(
        repo.clone(),
        policy(true, true).restrict_direct_peers(vec!["127.0.0.0/8".parse().unwrap()]),
    )
    .await;
    assert_eq!(
        send(address, Endpoint::C2bValidation, c2b("ACTIVE01", "100"))
            .await
            .1["ResultCode"],
        "0"
    );
    for account in ["EXPIRED1", "WRONG001", "STALEFX1", "VERIFIED"] {
        assert_eq!(
            send(address, Endpoint::C2bValidation, c2b(account, "100"))
                .await
                .1["ResultCode"],
            "C2B00012",
            "{account}"
        );
    }
    let mut masked = c2b("ACTIVE01", "100");
    masked["MSISDN"] = json!("2547***5678");
    assert_eq!(
        send(address, Endpoint::C2bValidation, masked).await.1["ResultCode"],
        "0"
    );
    let mut malformed = c2b("ACTIVE01", "100");
    malformed["TransTime"] = json!("20269999123456");
    assert_eq!(
        send(address, Endpoint::C2bConfirmation, malformed).await.0,
        400
    );
    assert_eq!(
        post(
            address,
            Endpoint::C2bValidation,
            &secret(Endpoint::C2bConfirmation),
            &c2b("ACTIVE01", "100").to_string(),
            ""
        )
        .await
        .0,
        404
    );
    server.abort();
    repo.close().await;
}

#[test]
fn configuration_requires_distinct_full_length_hex_secrets_and_bounded_limits() {
    let secrets = || {
        Endpoint::ALL
            .into_iter()
            .map(|e| (e, Secret::new(secret(e))))
            .collect::<BTreeMap<_, _>>()
    };
    let mut missing = secrets();
    missing.remove(&Endpoint::Stk);
    assert!(Policy::new(id("12345"), missing).is_err());
    let mut short = secrets();
    short.insert(Endpoint::Stk, Secret::new("short"));
    assert!(Policy::new(id("12345"), short).is_err());
    let mut duplicate = secrets();
    duplicate.insert(Endpoint::Stk, Secret::new(secret(Endpoint::B2c)));
    assert!(Policy::new(id("12345"), duplicate).is_err());
    assert!(
        policy(false, false)
            .with_limits(0, Duration::from_millis(500))
            .is_err()
    );
    assert!(
        policy(false, false)
            .with_limits(1024, Duration::from_secs(3))
            .is_err()
    );
}

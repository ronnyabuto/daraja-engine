use super::*;
use daraja_core::{
    crypto::{Environment, SecurityCredentialKey},
    payout::*,
};

fn remitter(account: &str) -> Remitter {
    Remitter::new(
        Environment::Sandbox,
        &SecurityCredentialKey::from_certificate(
            Environment::Sandbox,
            include_bytes!("../fixtures/rsa-cert.txt"),
        )
        .unwrap(),
        Secret::new("corporate-initiator"),
        &Secret::new("corporate-password"),
        id(account),
    )
    .unwrap()
}
fn request(destination: &str) -> PayoutRequest {
    PayoutRequest {
        caller_originator_conversation_id: id("caller-id-kept"),
        destination: Destination::parse(destination).unwrap(),
        amount: Amount::new(12300, Unit::KesMinor),
        result_url: Secret::new("https://example.invalid/payout/result-secret"),
        timeout_url: Secret::new("https://example.invalid/payout/timeout-secret"),
    }
}
fn capabilities() -> Capabilities {
    Capabilities {
        phone_01: true,
        buy_goods: true,
        pay_bill: true,
        b2b_caller_id: true,
        linked_account_sweep: true,
        transfer_mmf_to_utility: true,
    }
}
fn client(server: &Server) -> PayoutClient<ReqwestTransport> {
    PayoutClient::new(
        ReqwestTransport::new(Duration::from_millis(100), Duration::from_millis(400)).unwrap(),
        server.base.clone(),
        DisbursementApp::new(server.config("disbursement")).unwrap(),
        remitter("600000"),
        250000,
    )
    .unwrap()
}
struct PayoutHooks {
    state: Arc<Provider>,
    bodies: Mutex<Vec<String>>,
    acknowledgments: Mutex<Vec<(Id, Id, Id)>>,
    fail_prepare: bool,
    fail_ack: bool,
}
impl PayoutHooks {
    fn new(server: &Server) -> Self {
        server.state.require_prepared.store(true, Ordering::SeqCst);
        Self {
            state: server.state.clone(),
            bodies: Mutex::default(),
            acknowledgments: Mutex::default(),
            fail_prepare: false,
            fail_ack: false,
        }
    }
}
impl PayoutPersistence for PayoutHooks {
    async fn prepare(&self, payout: &PreparedPayout) -> Result<()> {
        if self.fail_prepare {
            return Err(ClientError::new(ErrorKind::Persistence));
        }
        let value: Value = serde_json::from_str(payout.wire_body.expose()).unwrap();
        assert_eq!(
            value["OriginatorConversationID"],
            payout.caller_originator_conversation_id.as_str()
        );
        self.bodies
            .lock()
            .unwrap()
            .push(payout.wire_body.expose().into());
        self.state.prepared.store(true, Ordering::SeqCst);
        Ok(())
    }
    async fn acknowledge(&self, ack: &PayoutAcknowledgment) -> Result<()> {
        if self.fail_ack {
            return Err(ClientError::new(ErrorKind::Persistence));
        }
        self.acknowledgments.lock().unwrap().push((
            ack.caller_originator_conversation_id.clone(),
            ack.provider.originator_conversation_id.clone(),
            ack.provider.conversation_id.clone(),
        ));
        Ok(())
    }
}
fn decrypt(value: &Value) {
    use base64::Engine;
    use rsa::pkcs8::DecodePrivateKey;
    let key = rsa::RsaPrivateKey::from_pkcs8_pem(include_str!("../fixtures/rsa-private-key.txt"))
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value.as_str().unwrap())
        .unwrap();
    assert_eq!(
        key.decrypt(rsa::Pkcs1v15Encrypt, &bytes).unwrap(),
        b"corporate-password"
    );
}

#[test]
fn destinations_are_normalized_and_ambiguous_or_out_of_range_forms_rejected() {
    for (input, expected) in [
        ("0712345678", "254712345678"),
        ("0112345678", "254112345678"),
        ("254712345678", "254712345678"),
        ("254112345678", "254112345678"),
    ] {
        let Destination::Phone(phone) = Destination::parse(input).unwrap() else {
            panic!()
        };
        assert_eq!(phone.expose(), expected);
        assert!(!format!("{phone:?}").contains(expected));
    }
    for valid in [
        "TILL:12345",
        "TILL:1234567",
        "PAYBILL:12345:A",
        "PAYBILL:123456:ABC_123-45678",
    ] {
        Destination::parse(valid).unwrap();
    }
    for bad in [
        "",
        "712345678",
        "+254712345678",
        "254212345678",
        "25471234567",
        "2547123456789",
        "0712 345678",
        " 0712345678",
        "0712345678\n",
        "０７１２３４５６７８",
        "till:123456",
        "TILL:000000",
        "TILL:1234",
        "TILL:12345678",
        "TILL:123456:abc",
        "PAYBILL:123456",
        "PAYBILL:1234:A",
        "PAYBILL:1234567:A",
        "PAYBILL:012345:A",
        "PAYBILL:123456:",
        "PAYBILL:123456:A:B",
        "PAYBILL:123456: A",
        "PAYBILL:123456:ABCDEFGHIJKLMN",
    ] {
        assert!(Destination::parse(bad).is_err(), "accepted {bad:?}");
    }
}

#[tokio::test]
async fn every_payout_command_has_exact_wire_identity_credentials_and_corporate_privacy() {
    let cases = [
        ("0712345678", "BusinessPayment", "254712345678", None),
        ("254712345678", "BusinessPayment", "254712345678", None),
        ("0112345678", "BusinessPayment", "254112345678", None),
        ("254112345678", "BusinessPayment", "254112345678", None),
        ("TILL:123456", "BusinessBuyGoods", "123456", Some(2)),
        (
            "PAYBILL:654321:INV_123",
            "BusinessPayBill",
            "654321",
            Some(4),
        ),
        (
            "PAYBILL:654321:FLOAT",
            "BusinessTransferFromMMFToUtility",
            "654321",
            Some(4),
        ),
    ];
    for (target, command, party_b, receiver) in cases {
        let mut reply = status_ack();
        reply["TransactionID"] = json!("ABC1234567");
        let server = Server::new(vec![Reply::Json(200, reply)]).await;
        let hooks = PayoutHooks::new(&server);
        let mut client = client(&server);
        client
            .configure_capabilities(
                capabilities(),
                Some(LinkedAccounts {
                    source: id("600000"),
                    utility: id("654321"),
                }),
            )
            .unwrap();
        let request = request(target);
        let ack = if command == "BusinessTransferFromMMFToUtility" {
            client.sweep(&request, &hooks).await
        } else {
            client.submit(&request, &hooks).await
        }
        .unwrap();
        assert_eq!(ack.caller_originator_conversation_id, id("caller-id-kept"));
        assert_eq!(ack.provider.originator_conversation_id, id("originator-1"));
        assert_eq!(ack.provider.outcome.transaction_id, Some(id("ABC1234567")));
        let seen = server.state.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].path,
            if receiver.is_none() {
                "/mpesa/b2c/v3/paymentrequest"
            } else {
                "/mpesa/b2b/v1/paymentrequest"
            }
        );
        assert_eq!(
            seen[0].headers["authorization"],
            "Bearer disbursement-token"
        );
        let actual = &seen[0].body;
        decrypt(&actual["SecurityCredential"]);
        let mut expected = json!({"OriginatorConversationID":"caller-id-kept", "SecurityCredential":actual["SecurityCredential"],"CommandID":command,"Amount":123,"PartyA":"600000","PartyB":party_b,"Remarks":"Corporate payout","ResultURL":request.result_url.expose(),"QueueTimeOutURL":request.timeout_url.expose()});
        if let Some(receiver) = receiver {
            expected["Initiator"] = json!("corporate-initiator");
            expected["SenderIdentifierType"] = json!(4);
            expected["RecieverIdentifierType"] = json!(receiver);
            expected["AccountReference"] = json!(if command == "BusinessBuyGoods" {
                "Payout"
            } else if command == "BusinessPayBill" {
                "INV_123"
            } else {
                "FLOAT"
            });
            assert!(!actual.to_string().contains("254712345678"));
            assert!(actual.get("Requester").is_none());
            assert!(actual.get("PhoneNumber").is_none());
        } else {
            expected["InitiatorName"] = json!("corporate-initiator");
            expected["Occassion"] = json!("Payout");
        }
        assert_eq!(actual, &expected);
        assert_eq!(hooks.bodies.lock().unwrap()[0], actual.to_string());
        assert_eq!(
            hooks.acknowledgments.lock().unwrap().as_slice(),
            &[(
                id("caller-id-kept"),
                id("originator-1"),
                id("conversation-1")
            )]
        );
    }
}

#[tokio::test]
async fn capabilities_and_linked_accounts_fail_closed_before_prepare_or_io() {
    let server = Server::new(vec![]).await;
    let hooks = PayoutHooks::new(&server);
    let mut client = client(&server);
    for target in ["0112345678", "TILL:123456", "PAYBILL:654321:A"] {
        let error = client.submit(&request(target), &hooks).await.unwrap_err();
        assert_eq!(error.disposition, Disposition::NotDispatched);
        assert_eq!(error.source.kind, ErrorKind::CapabilityDisabled);
    }
    let linked = || {
        Some(LinkedAccounts {
            source: id("600000"),
            utility: id("654321"),
        })
    };
    for (caps, accounts) in [
        (Capabilities::default(), linked()),
        (capabilities(), None),
        (
            Capabilities {
                transfer_mmf_to_utility: false,
                ..capabilities()
            },
            linked(),
        ),
        (
            Capabilities {
                linked_account_sweep: false,
                ..capabilities()
            },
            linked(),
        ),
        (
            Capabilities {
                b2b_caller_id: false,
                ..capabilities()
            },
            linked(),
        ),
    ] {
        client.configure_capabilities(caps, accounts).unwrap();
        assert_eq!(
            client
                .sweep(&request("PAYBILL:654321:FLOAT"), &hooks)
                .await
                .unwrap_err()
                .source
                .kind,
            ErrorKind::CapabilityDisabled
        );
    }
    client
        .configure_capabilities(capabilities(), linked())
        .unwrap();
    assert!(
        client
            .sweep(&request("PAYBILL:654322:FLOAT"), &hooks)
            .await
            .is_err()
    );
    assert!(client.sweep(&request("TILL:123456"), &hooks).await.is_err());
    assert!(
        client
            .configure_capabilities(
                capabilities(),
                Some(LinkedAccounts {
                    source: id("600001"),
                    utility: id("654321")
                })
            )
            .is_err()
    );
    assert!(
        client
            .configure_capabilities(
                capabilities(),
                Some(LinkedAccounts {
                    source: id("600000"),
                    utility: id("600000")
                })
            )
            .is_err()
    );
    assert_eq!(server.count(), 0);
    assert_eq!(server.state.auth_calls.load(Ordering::SeqCst), 0);
    assert!(hooks.bodies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn malformed_provider_errors_duplicates_timeouts_and_5xx_never_replay() {
    let cases = [
        (
            Reply::Json(
                500,
                json!({"errorCode":"500.002.1001","requestId":"duplicate-request","ConversationID":"dup-conversation"}),
            ),
            Disposition::Duplicate,
        ),
        (
            Reply::Json(200, json!({"ResponseCode":"1"})),
            Disposition::Uncertain,
        ),
        (
            Reply::Json(400, json!({"errorCode":"verified-reject"})),
            Disposition::ProvenRejection,
        ),
        (
            Reply::Json(500, json!({"errorCode":"verified-reject"})),
            Disposition::Uncertain,
        ),
        (Reply::Raw(500, "oops"), Disposition::Uncertain),
        (
            Reply::Json(
                503,
                json!({"errorCode":"503.001","OriginatorConversationID":"proxy-error"}),
            ),
            Disposition::Uncertain,
        ),
        (Reply::HeadersHang, Disposition::Uncertain),
        (Reply::BodyHang, Disposition::Uncertain),
        (Reply::Raw(200, "{}"), Disposition::Uncertain),
        (
            Reply::Raw(200, "{\"ResponseCode\":0,\"ResponseCode\":1}"),
            Disposition::Uncertain,
        ),
        (
            Reply::Json(
                200,
                json!({"ResponseCode":0,"ConversationID":"missing-originator"}),
            ),
            Disposition::Uncertain,
        ),
        (
            Reply::Json(401, json!({"errorCode":"401.001"})),
            Disposition::Uncertain,
        ),
    ];
    for target in [
        "0712345678",
        "TILL:123456",
        "PAYBILL:654321:A",
        "PAYBILL:654321:FLOAT",
    ] {
        for (reply, disposition) in cases.clone() {
            let server = Server::new(vec![reply]).await;
            let hooks = PayoutHooks::new(&server);
            let mut client = client(&server);
            client
                .configure_capabilities(
                    capabilities(),
                    Some(LinkedAccounts {
                        source: id("600000"),
                        utility: id("654321"),
                    }),
                )
                .unwrap();
            let command = match target {
                "0712345678" => Command::BusinessPayment,
                "TILL:123456" => Command::BusinessBuyGoods,
                "PAYBILL:654321:FLOAT" => Command::BusinessTransferFromMMFToUtility,
                _ => Command::BusinessPayBill,
            };
            client
                .configure_proven_rejections(command, vec![id("verified-reject")])
                .unwrap();
            let error = if target.ends_with(":FLOAT") {
                client.sweep(&request(target), &hooks).await
            } else {
                client.submit(&request(target), &hooks).await
            }
            .unwrap_err();
            assert_eq!(error.disposition, disposition);
            assert_eq!(
                error.caller_originator_conversation_id,
                id("caller-id-kept")
            );
            assert_eq!(server.count(), 1);
            assert!(hooks.acknowledgments.lock().unwrap().is_empty());
            assert_eq!(hooks.bodies.lock().unwrap().len(), 1);
            if disposition == Disposition::Duplicate {
                assert_eq!(error.source.outcome.provider_code, Some(id("500.002.1001")));
                assert_eq!(
                    error.source.outcome.request_id,
                    Some(id("duplicate-request"))
                );
                assert_eq!(
                    error.source.outcome.conversation_id,
                    Some(id("dup-conversation"))
                );
            }
        }
    }
}

#[tokio::test]
async fn payout_validation_and_persistence_failures_preserve_evidence_and_prevent_false_success() {
    let server = Server::new(vec![Reply::Json(200, status_ack())]).await;
    let mut hooks = PayoutHooks::new(&server);
    let client = client(&server);
    let mut req = request("0712345678");
    for amount in [
        Amount::new(0, Unit::KesMinor),
        Amount::new(1, Unit::KesMinor),
        Amount::new(1, Unit::Sat),
        Amount::new(25000100, Unit::KesMinor),
        Amount::new(u64::MAX, Unit::KesMinor),
    ] {
        req.amount = amount;
        assert_eq!(
            client.submit(&req, &hooks).await.unwrap_err().disposition,
            Disposition::NotDispatched
        );
    }
    req = request("0712345678");
    req.timeout_url = Secret::new("http://example.invalid/timeout");
    assert!(client.submit(&req, &hooks).await.is_err());
    req = request("0712345678");
    req.caller_originator_conversation_id = id(&"a".repeat(65));
    assert!(client.submit(&req, &hooks).await.is_err());
    req = request("0712345678");
    hooks.fail_prepare = true;
    assert_eq!(
        client.submit(&req, &hooks).await.unwrap_err().disposition,
        Disposition::NotDispatched
    );
    assert_eq!(server.count(), 0);
    hooks.fail_prepare = false;
    hooks.fail_ack = true;
    let error = client.submit(&req, &hooks).await.unwrap_err();
    assert_eq!(error.disposition, Disposition::Uncertain);
    assert_eq!(error.source.kind, ErrorKind::Persistence);
    assert_eq!(
        error.source.outcome.originator_conversation_id,
        Some(id("originator-1"))
    );
    assert_eq!(server.count(), 1);
}

struct BalanceHooks {
    state: Arc<Provider>,
    rows: Mutex<Vec<String>>,
    fail_prepare: bool,
    fail_ack: bool,
}
impl BalancePersistence for BalanceHooks {
    async fn prepare(&self, record: BalancePrepared<'_>) -> Result<()> {
        if self.fail_prepare {
            return Err(ClientError::new(ErrorKind::Persistence));
        }
        assert_eq!(record.created_at, FixedClock.now());
        assert_eq!(record.request.local_query_id, id("balance-local"));
        self.rows
            .lock()
            .unwrap()
            .push(record.wire_body.expose().into());
        self.state.prepared.store(true, Ordering::SeqCst);
        Ok(())
    }
    async fn acknowledge(&self, local: &Id, ack: &StatusAcknowledgment) -> Result<()> {
        assert_eq!(local, &id("balance-local"));
        assert_eq!(ack.originator_conversation_id, id("originator-1"));
        if self.fail_ack {
            return Err(ClientError::new(ErrorKind::Persistence));
        }
        self.rows.lock().unwrap().push("ack".into());
        Ok(())
    }
}
#[tokio::test]
async fn balance_uses_explicit_account_app_identifiers_and_async_persistence() {
    for collection in [false, true] {
        for till in [false, true] {
            let server = Server::new(vec![Reply::Json(200, status_ack())]).await;
            server.state.require_prepared.store(true, Ordering::SeqCst);
            let credentials = if collection {
                StatusCredentials::Collection(
                    CollectionApp::new(server.config("collection")).unwrap(),
                )
            } else {
                StatusCredentials::Disbursement(
                    DisbursementApp::new(server.config("disbursement")).unwrap(),
                )
            };
            let account = if till { "1234567" } else { "600000" };
            let mut client = BalanceClient::new(
                ReqwestTransport::new(Duration::from_millis(100), Duration::from_secs(1)).unwrap(),
                server.base.clone(),
                credentials,
                remitter(account),
                FixedClock,
            )
            .unwrap();
            let hooks = BalanceHooks {
                state: server.state.clone(),
                rows: Mutex::default(),
                fail_prepare: false,
                fail_ack: false,
            };
            let request = BalanceRequest {
                local_query_id: id("balance-local"),
                account: if till {
                    BalanceAccount::Till(id(account))
                } else {
                    BalanceAccount::Shortcode(id(account))
                },
                result_url: Secret::new("https://example.invalid/balance/result"),
                timeout_url: Secret::new("https://example.invalid/balance/timeout"),
            };
            if till {
                assert_eq!(
                    client.request(&request, &hooks).await.unwrap_err().kind,
                    ErrorKind::CapabilityDisabled
                );
                client.enable_till_balance();
            }
            client.request(&request, &hooks).await.unwrap();
            let seen = server.state.seen.lock().unwrap();
            assert_eq!(seen[0].path, "/mpesa/accountbalance/v1/query");
            assert_eq!(
                seen[0].headers["authorization"],
                format!(
                    "Bearer {}-token",
                    if collection {
                        "collection"
                    } else {
                        "disbursement"
                    }
                )
            );
            let body = &seen[0].body;
            decrypt(&body["SecurityCredential"]);
            assert_eq!(
                body,
                &json!({"CommandID":"AccountBalance","Initiator":"corporate-initiator","SecurityCredential":body["SecurityCredential"],"PartyA":account,"IdentifierType":if till {2} else {4},"Remarks":"Balance query","ResultURL":request.result_url.expose(),"QueueTimeOutURL":request.timeout_url.expose()})
            );
            assert_eq!(
                hooks.rows.lock().unwrap().as_slice(),
                &[body.to_string(), "ack".into()]
            );
        }
    }
}
#[test]
fn payout_receipts_amounts_and_balance_timeout_observations_are_preserved_without_finality() {
    let body = json!({"Result":{"OriginatorConversationID":"proxy","ConversationID":"conversation","ResultCode":0,"TransactionID":"QAB1234567","ResultParameters":{"ResultParameter":[{"Key":"TransactionReceipt","Value":"QAB1234567"},{"Key":"TransactionAmount","Value":"123.00"},{"Key":"ReceiverPartyPublicName","Value":"customer-sensitive-name"}]}}});
    let parsed = PayoutResult::parse(&body.to_string()).unwrap();
    assert_eq!(parsed.receipt.unwrap().as_str(), "QAB1234567");
    assert_eq!(parsed.amount, Some(Amount::new(12300, Unit::KesMinor)));
    assert!(!format!("{:?}", parsed.observation).contains("customer-sensitive-name"));
    let mut conflict = body.clone();
    conflict["Result"]["TransactionID"] = json!("OTHER12345");
    assert!(PayoutResult::parse(&conflict.to_string()).is_err());
    let mut conflict = body.clone();
    conflict["Result"]["ResultParameters"]["ResultParameter"]
        .as_array_mut()
        .unwrap()
        .push(json!({"Key":"Amount","Value":"124"}));
    assert!(PayoutResult::parse(&conflict.to_string()).is_err());
    let mut duplicate = body.clone();
    duplicate["Result"]["ResultParameters"]["ResultParameter"]
        .as_array_mut()
        .unwrap()
        .push(json!({"Key":"TransactionReceipt","Value":"QAB1234567"}));
    assert!(PayoutResult::parse(&duplicate.to_string()).is_err());
    let timeout = json!({"Result":{"OriginatorConversationID":"proxy","ConversationID":"conversation","ResultCode":"2001"}});
    assert_eq!(
        PayoutTimeout::parse(&timeout.to_string())
            .unwrap()
            .result_code,
        id("2001")
    );
    assert_eq!(
        BalanceTimeout::parse(&timeout.to_string())
            .unwrap()
            .result_code,
        id("2001")
    );
    let mut balance = timeout;
    balance["Result"]["ResultCode"] = json!(0);
    balance["Result"]["ResultParameters"] = json!({"ResultParameter":[{"Key":"AccountBalance","Value":"Utility Account|KES|123.00|123.00|0.00|0.00"}]});
    let parsed = BalanceResult::parse(&balance.to_string()).unwrap();
    assert_eq!(
        parsed.parameters["AccountBalance"].expose(),
        "Utility Account|KES|123.00|123.00|0.00|0.00"
    );
    assert!(parsed.observed_amount.is_none());
    assert!(parsed.receipt.is_none());
}

#[tokio::test]
async fn balance_errors_prepare_and_ack_failures_keep_queries_unresolved_without_retry() {
    for (reply, fail_prepare, fail_ack) in [
        (Reply::Json(200, status_ack()), true, false),
        (Reply::Json(200, status_ack()), false, true),
        (Reply::Raw(200, "{}"), false, false),
        (
            Reply::Raw(200, "{\"ResponseCode\":0,\"ResponseCode\":1}"),
            false,
            false,
        ),
        (Reply::Raw(500, "internal error"), false, false),
        (
            Reply::Json(503, json!({"errorCode":"503.001"})),
            false,
            false,
        ),
        (
            Reply::Json(400, json!({"errorCode":"500.002.1001"})),
            false,
            false,
        ),
        (Reply::HeadersHang, false, false),
        (Reply::BodyHang, false, false),
    ] {
        let server = Server::new(vec![reply]).await;
        server.state.require_prepared.store(true, Ordering::SeqCst);
        let client = BalanceClient::new(
            ReqwestTransport::new(Duration::from_millis(100), Duration::from_millis(300)).unwrap(),
            server.base.clone(),
            StatusCredentials::Collection(CollectionApp::new(server.config("collection")).unwrap()),
            remitter("600000"),
            FixedClock,
        )
        .unwrap();
        let hooks = BalanceHooks {
            state: server.state.clone(),
            rows: Mutex::default(),
            fail_prepare,
            fail_ack,
        };
        let mut req = BalanceRequest {
            local_query_id: id("balance-local"),
            account: BalanceAccount::Shortcode(id("600001")),
            result_url: Secret::new("https://example.invalid/result"),
            timeout_url: Secret::new("https://example.invalid/timeout"),
        };
        assert_eq!(
            client.request(&req, &hooks).await.unwrap_err().kind,
            ErrorKind::Input
        );
        req.account = BalanceAccount::Shortcode(id("600000"));
        let error = client.request(&req, &hooks).await.unwrap_err();
        assert_eq!(server.count(), if fail_prepare { 0 } else { 1 });
        assert_eq!(
            hooks.rows.lock().unwrap().len(),
            if fail_prepare { 0 } else { 1 }
        );
        if fail_ack {
            assert_eq!(error.kind, ErrorKind::Persistence);
            assert_eq!(error.outcome.conversation_id, Some(id("conversation-1")));
        }
    }
}

#[tokio::test]
async fn environment_certificates_app_origins_and_rejection_code_scope_are_enforced() {
    let server = Server::new(vec![Reply::Json(
        400,
        json!({"errorCode":"verified-reject"}),
    )])
    .await;
    let key = SecurityCredentialKey::from_certificate(
        Environment::Sandbox,
        include_bytes!("../fixtures/rsa-cert.txt"),
    )
    .unwrap();
    assert!(
        Remitter::new(
            Environment::Production,
            &key,
            Secret::new("operator"),
            &Secret::new("password"),
            id("600000")
        )
        .is_err()
    );
    let transport =
        || ReqwestTransport::new(Duration::from_secs(1), Duration::from_secs(2)).unwrap();
    let app = || DisbursementApp::new(server.config("disbursement")).unwrap();
    assert!(
        PayoutClient::new(
            transport(),
            "https://api.safaricom.co.ke/".parse().unwrap(),
            app(),
            remitter("600000"),
            250000
        )
        .is_err()
    );
    assert!(
        PayoutClient::new(
            transport(),
            "https://sandbox.safaricom.co.ke/".parse().unwrap(),
            app(),
            remitter("600000"),
            250000
        )
        .is_err()
    );
    let hooks = PayoutHooks::new(&server);
    let mut client = client(&server);
    for code in ["0", "500.002.1001", "500.001.1001"] {
        assert!(
            client
                .configure_proven_rejections(Command::BusinessPayment, vec![id(code)])
                .is_err()
        );
    }
    client.configure_capabilities(capabilities(), None).unwrap();
    client
        .configure_proven_rejections(Command::BusinessPayment, vec![id("verified-reject")])
        .unwrap();
    assert_eq!(
        client
            .submit(&request("TILL:123456"), &hooks)
            .await
            .unwrap_err()
            .disposition,
        Disposition::Uncertain
    );
    assert_eq!(server.count(), 1);
}

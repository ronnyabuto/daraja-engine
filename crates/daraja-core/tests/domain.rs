use std::{
    collections::BTreeMap,
    sync::{Arc, Barrier, Mutex},
};

use daraja_core::{
    Error, Timestamp,
    amount::{Amount, FxSnapshot, TariffSnapshot, Unit},
    identity::{Correlation, Id, ReceiptId, RequestKey, RequestPayload, RequestRegistry},
    inbound::{
        Accounting, Allocation, AttemptState, DepositAttestation, DepositRail, DepositState,
        EvidenceSource, InboundQuote, InboundState, QuoteTerms, ReceiptObservation, StkAttempt,
        VerifiedDeposit,
    },
    outbound::{
        Ambiguity, CdkPaymentState, DispatchRecord, FinalOutcome, OutboundState, Payout,
        PayoutAttestation,
    },
};

fn id(value: &str) -> Id {
    Id::new(value).unwrap()
}
fn receipt_id(value: &str) -> ReceiptId {
    ReceiptId::new(value).unwrap()
}
fn kes(value: u64) -> Amount {
    Amount::new(value, Unit::KesMinor)
}
fn tariff() -> TariffSnapshot {
    TariffSnapshot {
        version: id("tariff-v1"),
        fee: kes(2),
    }
}
fn terms() -> QuoteTerms {
    QuoteTerms {
        expected: kes(100),
        credit: kes(98),
        quote_expires_at: Timestamp(200),
        fx: None,
        tariff: tariff(),
        hold_until: Timestamp(0),
    }
}
fn quote_with(qid: &str, account: &str, terms: QuoteTerms) -> InboundQuote {
    InboundQuote::new(id(qid), id(account), id("12345"), terms).unwrap()
}
fn quote() -> InboundQuote {
    quote_with("quote", "CDK9F2A1", terms())
}
fn ledger(quote: InboundQuote) -> Accounting {
    let mut ledger = Accounting::default();
    ledger.insert_quote(quote).unwrap();
    ledger
}
fn deposit(receipt: &str, amount: u64, rail: DepositRail) -> VerifiedDeposit {
    VerifiedDeposit::attest(DepositAttestation {
        source: EvidenceSource::ReviewedStatement(id("synthetic-statement")),
        receipt: receipt_id(receipt),
        quote_id: Some(id("quote")),
        amount: kes(amount),
        shortcode: id("12345"),
        rail,
        received_at: Timestamp(30),
        verified_at: Timestamp(40),
    })
    .unwrap()
}
fn c2b() -> DepositRail {
    DepositRail::C2b {
        account_ref: id("CDK9F2A1"),
    }
}
fn stk_quote() -> InboundQuote {
    let mut quote = quote();
    quote
        .start_stk(StkAttempt::new(id("attempt"), id("caller"), Timestamp(0)).unwrap())
        .unwrap();
    quote
        .acknowledge_stk(&id("attempt"), id("merchant"), id("checkout"))
        .unwrap();
    quote
}
fn payload() -> RequestPayload {
    RequestPayload {
        operation: id("payout"),
        quote_id: id("quote"),
        destination: id("synthetic-destination"),
        amount: kes(100),
        tariff_version: id("tariff-v1"),
        options: BTreeMap::new(),
    }
}
fn payout() -> Payout {
    Payout::dispatching(
        DispatchRecord::new(
            id("operation"),
            payload(),
            b"synthetic-frozen-body".to_vec(),
            tariff(),
            kes(102),
        )
        .unwrap(),
        id("caller-request"),
        Timestamp(0),
        Timestamp(10),
        Timestamp(100),
    )
    .unwrap()
}
fn final_evidence(outcome: FinalOutcome) -> PayoutAttestation {
    PayoutAttestation {
        source: EvidenceSource::ReviewedStatement(id("synthetic-statement")),
        operation_id: id("operation"),
        payload_hash: payload().hash(),
        amount: kes(100),
        destination: id("synthetic-destination"),
        outcome,
    }
}

#[test]
fn checked_amounts_reject_overflow_underflow_currency_and_fractional_provider_kes() {
    assert_eq!(kes(u64::MAX).checked_add(kes(1)), Err(Error::Overflow));
    assert_eq!(kes(0).checked_sub(kes(1)), Err(Error::InvalidAmount));
    assert_eq!(
        kes(5).checked_add(Amount::new(5, Unit::Sat)),
        Err(Error::CurrencyMismatch)
    );
    assert_eq!(
        kes(5).checked_sub(Amount::new(5, Unit::Sat)),
        Err(Error::CurrencyMismatch)
    );
    assert_eq!(kes(101).whole_kes(), Err(Error::InvalidAmount));
    assert_eq!(kes(1200).whole_kes(), Ok(12));
    assert_eq!(
        Amount::new(100, Unit::Sat).whole_kes(),
        Err(Error::CurrencyMismatch)
    );
}

#[test]
fn fees_and_rounding_never_create_value_exhaustive_small_domain() {
    for amount in 1..100 {
        for fee in 0..amount {
            for denominator in 1..10 {
                let fx = FxSnapshot {
                    source: id("fx"),
                    numerator: 7,
                    denominator,
                    expires_at: Timestamp(100),
                };
                let net = kes(amount).checked_sub(kes(fee)).unwrap();
                let credit = fx.credit(net).unwrap();
                assert!(
                    u128::from(credit.value()) * u128::from(denominator)
                        <= u128::from(amount - fee) * 7
                );
                let mut t = terms();
                t.expected = kes(amount);
                t.tariff.fee = kes(fee);
                t.credit = credit;
                t.fx = Some(fx);
                if credit.value() == 0 {
                    assert_eq!(t.validate(), Err(Error::InvalidAmount));
                    continue;
                }
                t.validate().unwrap();
                t.credit = Amount::new(credit.value() + 1, Unit::Sat);
                assert_eq!(t.validate(), Err(Error::AmountMismatch));
            }
        }
    }
    let mut fx = FxSnapshot {
        source: id("fx"),
        numerator: u64::MAX,
        denominator: 1,
        expires_at: Timestamp(100),
    };
    assert_eq!(fx.credit(kes(u64::MAX)), Err(Error::Overflow));
    fx.denominator = 0;
    assert_eq!(fx.credit(kes(1)), Err(Error::InvalidAmount));
    let mut t = terms();
    t.tariff.fee = kes(101);
    assert_eq!(t.validate(), Err(Error::InvalidAmount));
    t = terms();
    t.credit = Amount::new(98, Unit::Sat);
    assert_eq!(t.validate(), Err(Error::CurrencyMismatch));
}

#[test]
fn idempotency_replay_preserves_operation_and_rejects_payload_changes() {
    let key = RequestKey {
        caller: id("caller"),
        operation: id("payout"),
        key: id("key"),
    };
    let mut registry = RequestRegistry::default();
    let original = registry
        .bind(key.clone(), &payload(), id("original"))
        .unwrap();
    assert_eq!(
        registry
            .bind(key.clone(), &payload(), id("replacement"))
            .unwrap(),
        original
    );
    let mut changed = payload();
    changed.amount = kes(101);
    assert_eq!(
        registry.bind(key.clone(), &changed, id("replacement")),
        Err(Error::IdempotencyConflict)
    );
    assert_eq!(
        registry.bind(key, &payload(), id("replacement")).unwrap(),
        original
    );
}

#[test]
fn canonical_hash_is_order_independent_and_binds_all_fields_unambiguously() {
    let mut a = payload();
    a.options.insert("b".into(), "2".into());
    a.options.insert("a".into(), "1".into());
    let mut b = payload();
    b.options.insert("a".into(), "1".into());
    b.options.insert("b".into(), "2".into());
    assert_eq!(a.hash(), b.hash());
    let mut variants = vec![];
    let mut p = a.clone();
    p.operation = id("different");
    variants.push(p);
    let mut p = a.clone();
    p.quote_id = id("different");
    variants.push(p);
    let mut p = a.clone();
    p.destination = id("different");
    variants.push(p);
    let mut p = a.clone();
    p.amount = kes(101);
    variants.push(p);
    let mut p = a.clone();
    p.amount = Amount::new(100, Unit::Sat);
    variants.push(p);
    let mut p = a.clone();
    p.tariff_version = id("different");
    variants.push(p);
    let mut p = a.clone();
    p.options.insert("a".into(), "different".into());
    variants.push(p);
    for changed in variants {
        assert_ne!(a.hash(), changed.hash());
    }
    a.options = BTreeMap::from([("ab".into(), "c".into())]);
    b.options = BTreeMap::from([("a".into(), "bc".into())]);
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn spoofed_observation_does_not_poison_confirmed_receipt_slot() {
    let mut ledger = ledger(quote());
    ledger.observe(ReceiptObservation {
        observation_id: id("spoof"),
        claimed_receipt: receipt_id("receipt"),
        claimed_quote: Some(id("wrong-quote")),
        claimed_amount: kes(999),
        rail: c2b(),
        observed_at: Timestamp(10),
    });
    assert!(ledger.receipt(&receipt_id("receipt")).is_none());
    assert_eq!(
        ledger.quote(&id("quote")).unwrap().state(),
        InboundState::Issued
    );
    ledger.confirm(deposit("receipt", 100, c2b())).unwrap();
    let event = ledger.settle(&id("quote"), Timestamp(50)).unwrap();
    assert_eq!(event.credit, kes(98));
    assert_eq!(ledger.events().len(), 1);
    assert_eq!(ledger.observations().len(), 1);
    assert_eq!(
        ledger.confirm(deposit("receipt", 100, c2b())),
        Err(Error::DuplicateReceipt)
    );
    assert_eq!(
        ledger.settle(&id("quote"), Timestamp(60)),
        Err(Error::InvalidTransition)
    );
    assert_eq!(
        ledger.quote(&id("quote")).unwrap().state(),
        InboundState::Paid
    );
}

#[test]
fn bad_binding_does_not_mutate_quote_or_consume_receipt() {
    let mut ledger = ledger(quote());
    for mutate in 0..4 {
        let mut e = deposit("receipt", 100, c2b()).evidence().clone();
        match mutate {
            0 => e.shortcode = id("wrong"),
            1 => {
                e.rail = DepositRail::Stk {
                    checkout_request: id("unknown"),
                }
            }
            2 => {
                e.rail = DepositRail::C2b {
                    account_ref: id("WRONGREF"),
                }
            }
            _ => e.quote_id = Some(id("missing")),
        }
        assert!(ledger.confirm(VerifiedDeposit::attest(e).unwrap()).is_err());
        assert!(ledger.receipt(&receipt_id("receipt")).is_none());
        assert!(ledger.events().is_empty());
        assert_eq!(
            ledger.quote(&id("quote")).unwrap().state(),
            InboundState::Issued
        );
    }
    ledger.confirm(deposit("receipt", 100, c2b())).unwrap();
}

#[test]
fn distinct_topups_settle_once_with_all_receipts() {
    let mut ledger = ledger(quote());
    assert_eq!(
        ledger.confirm(deposit("first", 40, c2b())).unwrap(),
        DepositState::UnderpaidHeld
    );
    assert_eq!(
        ledger.settle(&id("quote"), Timestamp(50)),
        Err(Error::AmountMismatch)
    );
    assert!(ledger.events().is_empty());
    ledger.confirm(deposit("second", 60, c2b())).unwrap();
    let event = ledger.settle(&id("quote"), Timestamp(50)).unwrap();
    assert_eq!(
        event.receipts,
        vec![receipt_id("first"), receipt_id("second")]
    );
    assert_eq!(event.credit, kes(98));
    for receipt in event.receipts {
        assert_eq!(
            ledger.receipt(&receipt).unwrap().state,
            DepositState::Settled
        );
    }
}

#[test]
fn concurrent_rails_before_settlement_hold_excess_in_both_orders() {
    for reverse in [false, true] {
        let mut ledger = ledger(stk_quote());
        let stk = deposit(
            "stkreceipt",
            100,
            DepositRail::Stk {
                checkout_request: id("checkout"),
            },
        );
        let manual = deposit("c2breceipt", 100, c2b());
        let deposits = if reverse {
            [manual, stk]
        } else {
            [stk, manual]
        };
        for deposit in deposits {
            ledger.confirm(deposit).unwrap();
        }
        assert_eq!(
            ledger.quote(&id("quote")).unwrap().state(),
            InboundState::OverpaidHeld
        );
        assert_eq!(
            ledger.settle(&id("quote"), Timestamp(50)),
            Err(Error::AmountMismatch)
        );
        assert!(ledger.events().is_empty());
    }
}

#[test]
fn racing_rails_can_never_credit_twice() {
    let ledger = Arc::new(Mutex::new(ledger(stk_quote())));
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = [
        deposit(
            "stk",
            100,
            DepositRail::Stk {
                checkout_request: id("checkout"),
            },
        ),
        deposit("manual", 100, c2b()),
    ]
    .into_iter()
    .map(|deposit| {
        let ledger = ledger.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            let mut ledger = ledger.lock().unwrap();
            ledger.confirm(deposit).unwrap();
            ledger.settle(&id("quote"), Timestamp(50))
        })
    })
    .collect();
    let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let ledger = ledger.lock().unwrap();
    assert_eq!(ledger.events().len(), 1);
    let records = [
        ledger.receipt(&receipt_id("stk")).unwrap(),
        ledger.receipt(&receipt_id("manual")).unwrap(),
    ];
    assert_eq!(
        records
            .iter()
            .filter(|r| r.state == DepositState::Settled)
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|r| r.allocation == Allocation::Exception)
            .count(),
        1
    );
}

#[test]
fn receipt_cannot_fund_another_quote() {
    let mut ledger = ledger(quote());
    ledger
        .insert_quote(quote_with("other", "CDK9F2A2", terms()))
        .unwrap();
    ledger.confirm(deposit("receipt", 100, c2b())).unwrap();
    let mut e = deposit("receipt", 100, c2b()).evidence().clone();
    e.quote_id = Some(id("other"));
    e.rail = DepositRail::C2b {
        account_ref: id("CDK9F2A2"),
    };
    assert_eq!(
        ledger.confirm(VerifiedDeposit::attest(e).unwrap()),
        Err(Error::DuplicateReceipt)
    );
    assert_eq!(
        ledger.settle(&id("other"), Timestamp(50)),
        Err(Error::AmountMismatch)
    );
}

#[test]
fn overflow_confirmation_is_atomic_and_recoverable() {
    let mut t = terms();
    t.expected = kes(u64::MAX);
    t.credit = kes(u64::MAX - 2);
    let mut ledger = ledger(quote_with("quote", "CDK9F2A1", t));
    ledger
        .confirm(deposit("first", u64::MAX - 1, c2b()))
        .unwrap();
    assert_eq!(
        ledger.confirm(deposit("second", 2, c2b())),
        Err(Error::Overflow)
    );
    assert!(ledger.receipt(&receipt_id("second")).is_none());
    assert!(ledger.events().is_empty());
    assert_eq!(
        ledger.quote(&id("quote")).unwrap().state(),
        InboundState::UnderpaidHeld
    );
    ledger.confirm(deposit("second", 1, c2b())).unwrap();
    ledger.settle(&id("quote"), Timestamp(50)).unwrap();
}

#[test]
fn quote_expiry_fx_expiry_and_settlement_hold_are_separate() {
    let mut t = terms();
    t.hold_until = Timestamp(70);
    let mut ledger = ledger(quote_with("quote", "CDK9F2A1", t));
    assert_eq!(
        ledger.confirm(deposit("receipt", 100, c2b())).unwrap(),
        DepositState::SettlementHeld
    );
    assert_eq!(
        ledger.settle(&id("quote"), Timestamp(69)),
        Err(Error::SettlementHeld)
    );
    assert!(ledger.events().is_empty());
    assert_eq!(
        ledger.quote(&id("quote")).unwrap().state(),
        InboundState::SettlementHeld
    );
    ledger.settle(&id("quote"), Timestamp(70)).unwrap();
    let mut t = terms();
    t.fx = Some(FxSnapshot {
        source: id("fx"),
        numerator: 3,
        denominator: 2,
        expires_at: Timestamp(60),
    });
    t.credit = Amount::new(147, Unit::Sat);
    let mut fx_ledger = self::ledger(quote_with("quote", "CDK9F2A1", t));
    fx_ledger.confirm(deposit("receipt", 100, c2b())).unwrap();
    assert_eq!(
        fx_ledger.settle(&id("quote"), Timestamp(60)),
        Err(Error::FxExpired)
    );
    assert!(fx_ledger.events().is_empty());
    let mut late_ledger = self::ledger(quote());
    let mut e = deposit("late", 100, c2b()).evidence().clone();
    e.received_at = Timestamp(200);
    e.verified_at = Timestamp(201);
    assert_eq!(
        late_ledger
            .confirm(VerifiedDeposit::attest(e).unwrap())
            .unwrap(),
        DepositState::LateHeld
    );
    assert_eq!(
        late_ledger.settle(&id("quote"), Timestamp(210)),
        Err(Error::AmountMismatch)
    );
}

#[test]
fn expired_fx_and_orphan_value_remain_in_exception_custody() {
    let mut t = terms();
    t.fx = Some(FxSnapshot {
        source: id("fx"),
        numerator: 1,
        denominator: 1,
        expires_at: Timestamp(40),
    });
    t.credit = Amount::new(98, Unit::Sat);
    let mut ledger = ledger(quote_with("quote", "CDK9F2A1", t));
    assert_eq!(
        ledger.confirm(deposit("expired", 100, c2b())).unwrap(),
        DepositState::FxExpiredHeld
    );
    let mut e = deposit("orphan", 100, c2b()).evidence().clone();
    e.quote_id = None;
    assert_eq!(
        ledger.confirm(VerifiedDeposit::attest(e).unwrap()).unwrap(),
        DepositState::OrphanUnclaimed
    );
    assert!(ledger.events().is_empty());
    assert_eq!(
        ledger.receipt(&receipt_id("expired")).unwrap().allocation,
        Allocation::Exception
    );
}

#[test]
fn multiple_stk_attempts_have_independent_ids_timers_and_no_implicit_credit() {
    let mut q = stk_quote();
    assert_eq!(
        q.start_stk(StkAttempt::new(id("second"), id("caller"), Timestamp(34)).unwrap()),
        Err(Error::SettlementHeld)
    );
    q.start_stk(StkAttempt::new(id("second"), id("caller-2"), Timestamp(35)).unwrap())
        .unwrap();
    q.stk_unknown(&id("second")).unwrap();
    q.acknowledge_stk(&id("second"), id("merchant-2"), id("checkout-2"))
        .unwrap();
    q.finish_stk(&id("attempt"), false).unwrap();
    assert_eq!(q.stk_unknown(&id("attempt")), Err(Error::InvalidTransition));
    q.finish_stk(&id("second"), true).unwrap();
    assert_eq!(q.attempts().len(), 2);
    assert_eq!(q.attempts()[0].state(), AttemptState::Failed);
    assert_eq!(q.attempts()[1].state(), AttemptState::Completed);
    assert_eq!(q.state(), InboundState::StkSent);
    assert_eq!(q.attempts()[1].next_poll_at(), Timestamp(70));
    let mut ledger = ledger(q);
    ledger.confirm(deposit("manual", 100, c2b())).unwrap();
    ledger.settle(&id("quote"), Timestamp(50)).unwrap();
    assert!(StkAttempt::new(id("overflow"), id("caller"), Timestamp(u64::MAX)).is_err());
    let mut q = quote();
    assert_eq!(
        q.start_stk(StkAttempt::new(id("expired"), id("caller"), Timestamp(200)).unwrap()),
        Err(Error::QuoteExpired)
    );
}

#[test]
fn all_ambiguities_and_manual_escalation_keep_cdk_pending() {
    for reason in [
        Ambiguity::TransportError,
        Ambiguity::Http500,
        Ambiguity::Http503,
        Ambiguity::Timeout,
        Ambiguity::QueueTimeout,
        Ambiguity::WorkerRecovery,
    ] {
        let mut p = payout();
        let frozen = p.dispatch().clone();
        p.ambiguous(reason).unwrap();
        assert_eq!(p.state(), OutboundState::UnknownInFlight);
        assert_eq!(p.state().cdk_state(), CdkPaymentState::Pending);
        assert_eq!(p.escalate(Timestamp(99)), Err(Error::InvalidTransition));
        p.escalate(Timestamp(u64::MAX)).unwrap();
        assert_eq!(p.state(), OutboundState::ManualReconciliationRequired);
        p.ambiguous(reason).unwrap();
        p.duplicate_observed().unwrap();
        p.acknowledge(id("proxy"), id("conversation")).unwrap();
        assert_eq!(p.state(), OutboundState::ManualReconciliationRequired);
        assert_eq!(p.state().cdk_state(), CdkPaymentState::Pending);
        assert_eq!(p.dispatch(), &frozen);
    }
}

#[test]
fn definitive_evidence_resolves_from_every_nonterminal_state_and_terminal_is_final() {
    for state in 0..4 {
        for outcome in [
            FinalOutcome::Paid {
                receipt: receipt_id("receipt"),
            },
            FinalOutcome::DefinitivelyNotExecuted {
                evidence_record: id("nonexecution-proof"),
            },
        ] {
            let mut p = payout();
            match state {
                1 => p.ambiguous(Ambiguity::Timeout).unwrap(),
                2 => p.acknowledge(id("proxy"), id("conversation")).unwrap(),
                3 => p.escalate(Timestamp(100)).unwrap(),
                _ => (),
            }
            let evidence = final_evidence(outcome.clone());
            p.resolve(evidence.clone()).unwrap();
            let expected = if matches!(outcome, FinalOutcome::Paid { .. }) {
                CdkPaymentState::Paid
            } else {
                CdkPaymentState::Failed
            };
            assert_eq!(p.state().cdk_state(), expected);
            let terminal = p.clone();
            assert_eq!(
                p.ambiguous(Ambiguity::WorkerRecovery),
                Err(Error::InvalidTransition)
            );
            assert_eq!(p.duplicate_observed(), Err(Error::InvalidTransition));
            assert_eq!(
                p.acknowledge(id("proxy"), id("conversation")),
                Err(Error::InvalidTransition)
            );
            assert_eq!(p.escalate(Timestamp(1000)), Err(Error::InvalidTransition));
            assert_eq!(p.resolve(evidence), Err(Error::InvalidTransition));
            assert_eq!(p, terminal);
        }
    }
}

#[test]
fn mismatched_final_evidence_cannot_release_proofs() {
    for field in 0..4 {
        let mut p = payout();
        let mut e = final_evidence(FinalOutcome::DefinitivelyNotExecuted {
            evidence_record: id("proof"),
        });
        match field {
            0 => e.operation_id = id("other"),
            1 => {
                let mut changed = payload();
                changed.quote_id = id("other");
                e.payload_hash = changed.hash();
            }
            2 => e.amount = Amount::new(100, Unit::Sat),
            _ => e.destination = id("other"),
        }
        assert_eq!(p.resolve(e), Err(Error::EvidenceMismatch));
        assert_eq!(p.state().cdk_state(), CdkPaymentState::Pending);
        assert!(p.final_evidence().is_none());
    }
}

#[test]
fn correlation_records_caller_proxy_and_provider_without_equating_them() {
    let mut p = payout();
    p.acknowledge(id("proxy"), id("provider")).unwrap();
    assert_eq!(p.correlation().caller_request, id("caller-request"));
    assert_eq!(p.correlation().originator_conversation, id("operation"));
    let saved = p.clone();
    assert_eq!(
        p.acknowledge(id("different"), id("provider")),
        Err(Error::EvidenceMismatch)
    );
    assert_eq!(p, saved);
    p.ambiguous(Ambiguity::Timeout).unwrap();
    assert_eq!(p.state(), OutboundState::Pending);
    let mut c = Correlation {
        caller_request: id("caller"),
        originator_conversation: id("originator"),
        provider_originator: None,
        provider_conversation: None,
    };
    c.acknowledge(id("proxy"), id("provider")).unwrap();
    assert_eq!(
        c.acknowledge(id("proxy"), id("other")),
        Err(Error::EvidenceMismatch)
    );
}

#[test]
fn payout_reservation_and_tariff_cannot_be_underfunded_or_mismatched() {
    assert_eq!(
        DispatchRecord::new(id("operation"), payload(), vec![1], tariff(), kes(100)),
        Err(Error::AmountMismatch)
    );
    let mut t = tariff();
    t.version = id("other");
    assert_eq!(
        DispatchRecord::new(id("operation"), payload(), vec![1], t, kes(102)),
        Err(Error::EvidenceMismatch)
    );
    let mut p = payload();
    p.amount = kes(u64::MAX);
    assert_eq!(
        DispatchRecord::new(id("operation"), p, vec![1], tariff(), kes(u64::MAX)),
        Err(Error::Overflow)
    );
}

#[test]
fn debug_does_not_expose_wire_body_or_sensitive_options() {
    let mut p = payload();
    p.options.insert("callback".into(), "secret-url".into());
    let dispatch = DispatchRecord::new(
        id("sensitive-id"),
        p,
        b"secret-body".to_vec(),
        tariff(),
        kes(102),
    )
    .unwrap();
    let debug = format!("{dispatch:?}");
    assert!(!debug.contains("secret"));
    assert!(!format!("{:?}", dispatch.payload()).contains("secret-url"));
}

#[test]
fn canonical_receipt_spelling_cannot_bypass_deduplication() {
    assert_eq!(receipt_id("AbC123"), receipt_id("ABC123"));
    assert!(ReceiptId::new("").is_err());
    assert!(ReceiptId::new(" ABC123").is_err());
    assert!(ReceiptId::new("ＡBC123").is_err());
    let mut ledger = ledger(quote());
    ledger.confirm(deposit("AbC123", 100, c2b())).unwrap();
    assert_eq!(
        ledger.confirm(deposit("abc123", 100, c2b())),
        Err(Error::DuplicateReceipt)
    );
}

#[test]
fn accounting_owner_preserves_inbound_terminal_state_and_unique_quote_identity() {
    let mut ledger = ledger(quote());
    assert_eq!(ledger.insert_quote(quote()), Err(Error::DuplicateIdentity));
    assert_eq!(
        ledger.insert_quote(quote_with("different", "CDK9F2A1", terms())),
        Err(Error::DuplicateIdentity)
    );
    ledger
        .start_stk(
            &id("quote"),
            StkAttempt::new(id("attempt"), id("caller"), Timestamp(0)).unwrap(),
        )
        .unwrap();
    ledger
        .acknowledge_stk(&id("quote"), &id("attempt"), id("merchant"), id("checkout"))
        .unwrap();
    ledger.confirm(deposit("receipt", 100, c2b())).unwrap();
    ledger.settle(&id("quote"), Timestamp(50)).unwrap();
    assert_eq!(
        ledger.start_stk(
            &id("quote"),
            StkAttempt::new(id("second"), id("caller"), Timestamp(60)).unwrap()
        ),
        Err(Error::InvalidTransition)
    );
    assert_eq!(
        ledger.acknowledge_stk(&id("quote"), &id("attempt"), id("merchant"), id("checkout")),
        Err(Error::InvalidTransition)
    );
    ledger
        .confirm(deposit(
            "extra",
            100,
            DepositRail::Stk {
                checkout_request: id("checkout"),
            },
        ))
        .unwrap();
    assert_eq!(
        ledger.quote(&id("quote")).unwrap().state(),
        InboundState::Paid
    );
    assert_eq!(
        ledger.receipt(&receipt_id("extra")).unwrap().allocation,
        Allocation::Exception
    );
    assert_eq!(ledger.events().len(), 1);
}

#[test]
fn unverified_or_invalid_value_cannot_settle() {
    let mut ledger = ledger(quote());
    assert_eq!(
        ledger.settle(&id("quote"), Timestamp(u64::MAX)),
        Err(Error::AmountMismatch)
    );
    for amount in [kes(0), Amount::new(100, Unit::Sat)] {
        let mut evidence = deposit("receipt", 100, c2b()).evidence().clone();
        evidence.amount = amount;
        assert!(VerifiedDeposit::attest(evidence).is_err());
    }
    ledger.confirm(deposit("receipt", 100, c2b())).unwrap();
    assert_eq!(
        ledger.settle(&id("quote"), Timestamp(39)),
        Err(Error::EvidenceMismatch)
    );
    assert!(ledger.events().is_empty());
    ledger.settle(&id("quote"), Timestamp(40)).unwrap();
}

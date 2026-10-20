# PRD requirement traceability

Planning baseline extracted from docs/spec/prd-v1.0.pdf. Stage 01 checked the relevant PRD sections against these rows; no scope was dropped; add rows rather than silently dropping source requirements. Stage 28 audits every row against the source.

All statuses start NOT_STARTED. Statuses: NOT_STARTED, IN_PROGRESS, DONE (implemented with local evidence), BLOCKED, UNSUPPORTED. Local DONE does not imply live verification. External gate evidence is tracked separately in docs/verification-gates.md once created.

| ID | PRD section | Required behavior | Stages | Status | Implementation / test / evidence |
|---|---|---|---|---|---|
| R01 | 1, 5.1 | Single Rust 2024 workspace; core library, processor library/binary, sidecar binary | 00 | DONE | Root Cargo.toml; crates/{daraja-core,cdk-daraja-processor,daraja-daemon}/Cargo.toml and src; evidence/00-workspace.md: workspace build, fmt, tests, clippy |
| R02 | 1, 2.1 | Dual MIT/Apache-2.0 licensing and self-hosted credential ownership | 00,26 | IN_PROGRESS | Stage 00: LICENSE-MIT, LICENSE-APACHE; config.example.toml; .gitignore; scripts/check-foundation.py; evidence/00-workspace.md. Operator packaging remains stage 26. |
| R03 | 4, 5.1, 7 Phase 0 | Exact pinned CDK service/protobuf and all seven methods | 00,01,17,18 | IN_PROGRESS | Stage 00: docs/protocol-contract.md; vendored proto and upstream licenses; processor build.rs and tests/wire_contract.rs; evidence/00-workspace.md. Stage 01 accepts exact WaitPaymentEvent and records protocol version/header, source traces and loopback gRPC tests (tests/grpc_contract.rs); evidence/01-protocol-gates.md. Stages 17/18 service implementation remains. |
| R04 | 4, 7 Phase 0 | NUT-04/NUT-05 custom mpesa and KES/sat compatibility evidence | 01,19 | IN_PROGRESS | Stage 01: docs/protocol-contract.md pinned NUT/CDK types; scripts/check-cdk-upstream.py; wire_contract.rs and grpc_contract.rs; evidence/01-protocol-gates.md. Actual custom mint routes/keysets/FX remain gated through stage 19. |
| R05 | 1, 5.3 | Real CDK proof lock semantics across Pending, errors and recovery | 01,18,19 | IN_PROGRESS | Stage 01: scripts/check-cdk-upstream.py executes real mint regression tests; grpc_contract.rs verifies transport; protocol-contract.md traces error/recovery source. Combined processor/mint gRPC faults and durable process restart remain gated for stages 18/19; evidence/01-protocol-gates.md. |
| R06 | 5.3, 7 Phase 0 | Independent B2C/B2B idempotency guarantees and gated probing | 01,13,27 | IN_PROGRESS | Stage 01: scripts/sandbox-probe.py; test-protocol-gates.py tests ID and entire body equality, authorization, redaction and no retry; docs/verification-gates.{md,json}; config defaults false. BLOCKED external evidence separately for B2C v3/B2B v1; evidence/01-protocol-gates.md. |
| R07 | 2, 7 Phase 0 | Provider prerequisites and legal claims verified as deployment gates | 01,27 | IN_PROGRESS | Stage 01: docs/verification-gates.md official legal sources, final LN 134 Fifth Schedule correction, operator/counsel tasks and provider onboarding gates. Deployment clearance BLOCKED pending operator/counsel evidence; evidence/01-protocol-gates.md. |
| R08 | 4, 5 | Explicit state machines and exact amounts/units | 02 | DONE | Stage 02: daraja-core src/{amount,identity,inbound,outbound}.rs; docs/state-machines.md transition/evidence tables; tests/domain.rs (transitions, terminal rejection, exact units, fee/FX properties, expiry/holds); evidence/02-domain.md. Local domain acceptance only; external gates unchanged. |
| R09 | 4, 5.2, 5.3 | ACID schema, unique identities and safe state transitions | 02,03 | IN_PROGRESS | Stage 02: guarded quote/attempt/payout transitions, private immutable dispatch data and single-owner accounting aggregate; tests/domain.rs terminal, rollback, unique quote/account and competing-rail cases; docs/state-machines.md; evidence/02-domain.md. File-backed ACID schema, CAS and competing SQL writers remain stage 03. |
| R10 | 5.2, 5.3 | Receipt deduplication/allocation, request idempotency and payload identity | 02,03 | IN_PROGRESS | Stage 02: identity.rs caller-scoped RequestKey, canonical SHA-256 payload binding, normalized ReceiptId; inbound.rs separates observations from confirmed allocations. tests/domain.rs spoof-first recovery, same-key/different-payload, top-ups, duplicate/cross-quote receipts and both rail orderings; evidence/02-domain.md. Database enforcement remains stage 03. |
| R11 | 5.2, 5.3 | Durable jobs, events, correlation and restart/crash recovery | 02,03,09,12,13 | IN_PROGRESS | Stage 02: caller/provider STK/payout correlation, frozen dispatch identity/body, attempt/poll/escalation timing, settlement plus event aggregate contract; tests/domain.rs correlation, ambiguity/manual Pending, single-event and failure rollback; docs/state-machines.md; evidence/02-domain.md. Durable jobs/outbox and real restart/crash recovery remain stages 03/09/12/13. |
| R12 | 5.1 | Isolated OAuth actors, proactive refresh, STK EAT password and RSA credentials | 04 | NOT_STARTED | — |
| R13 | 1, 5.2 | STK processrequest and stkpushquery clients | 05 | NOT_STARTED | — |
| R14 | 1, 5.2 | C2B RegisterURL with validation and confirmation wiring | 05,07 | NOT_STARTED | — |
| R15 | 1, 5.2, 5.3 | Asynchronous TransactionStatus query/result/timeout correlation | 05,09,13 | NOT_STARTED | — |
| R16 | 5.3 | B2C v3 BusinessPayment and Kenyan phone destination forms | 06,12 | NOT_STARTED | — |
| R17 | 5.3 | B2B v1 BusinessBuyGoods and BusinessPayBill wire contracts/destinations | 06,12 | NOT_STARTED | — |
| R18 | 1, 6 | AccountBalance request/result/timeout handling | 06,14 | NOT_STARTED | — |
| R19 | 6 | Optional BusinessTransferFromMMFToUtility with linked accounts | 06,14 | NOT_STARTED | — |
| R20 | 4, 5.1, 5.2 | Secret/quote binding, DTO/body limits, optional CIDR policy and redaction | 07,25 | NOT_STARTED | — |
| R21 | 5.2.4 | Synchronous C2B account/amount rejection, enabled-mode semantics | 07,10 | NOT_STARTED | — |
| R22 | 5.2.1 | Eight-character reference, durable quote TTL and optional STK dispatch | 08 | NOT_STARTED | — |
| R23 | 5.2.1 | Immediate dual-rail fallback and enforced 35-second resend cooldown | 08,22,23 | NOT_STARTED | — |
| R24 | 5.2.2 | Four verification gates for every deposit, real field/evidence constraints | 09 | NOT_STARTED | — |
| R25 | 5.2.3 | STK polls at 35/65/110 seconds and spike-arrest backoff | 09 | NOT_STARTED | — |
| R26 | 5.2.3 | Atomic verified settlement and durable incoming notification/replay | 09,17 | NOT_STARTED | — |
| R27 | 5.2.4 | UNDERPAID_HELD and distinct verified top-up allocation | 10 | NOT_STARTED | — |
| R28 | 5.2.4 | Refunds net of B2C fee, authorization and no double allocation | 10,15 | NOT_STARTED | — |
| R29 | 5.2.4 | OVERPAID_HELD, ORPHAN_UNCLAIMED and late KES/sat policies | 10 | NOT_STARTED | — |
| R30 | 5.2.4 | Reversal prerequisite, velocity limits and high-value settlement holds | 10,15 | NOT_STARTED | — |
| R31 | 4, 5.3.1 | Distinct configured B2C/B2B tariffs, frozen quotes and fee accounting | 11 | NOT_STARTED | — |
| R32 | 5.2.1, 5.2.4 | Pluggable live-capable FX adapter, strict TTL, exact KES/sat conversion | 11 | NOT_STARTED | — |
| R33 | 5.3.2 | Persist DISPATCHING and deterministic OriginatorConversationID before I/O | 12 | NOT_STARTED | — |
| R34 | 5.3.2 | Store caller and provider IDs; concurrent/early-callback recovery | 12 | NOT_STARTED | — |
| R35 | 5.3.3 | 500/503/timeouts/QueueTimeOutURL stay Pending; evidence-gated same-ID probe | 13 | NOT_STARTED | — |
| R36 | 5.3.3 | Status reconciliation, bounded polling/manual TTL, audited evidence resolution | 13,15 | NOT_STARTED | — |
| R37 | 6 | Separate float telemetry and named Prometheus balance metrics | 14 | NOT_STARTED | — |
| R38 | 6 | Pre-quote minimum-float breaker with fees, freshness and reservations | 14 | NOT_STARTED | — |
| R39 | 6 | Linked-account sweep opt-in and unlinked low-float alerts | 14 | NOT_STARTED | — |
| R40 | 5.2.4, 5.3.3, 6 | Authenticated operator exceptions/refunds/re-quote/reconciliation runbook | 15 | NOT_STARTED | — |
| R41 | 4, 5.1 | Optional PostgreSQL backend with tested SQLite parity | 16 | NOT_STARTED | — |
| R42 | 5.1, 5.2 | CDK GetSettings/CreatePayment/CheckIncomingPayment/WaitIncomingPayment | 17 | NOT_STARTED | — |
| R43 | 5.1, 5.3 | CDK GetPaymentQuote/MakePayment/CheckOutgoingPayment | 18 | NOT_STARTED | — |
| R44 | 1, 5.3 | Pending proof safety and actual M-Pesa receipt as proof/preimage | 18,19 | NOT_STARTED | — |
| R45 | 1, 7 Phase 2 | Real pinned mint + local provider integration and bearer transfer | 19 | NOT_STARTED | — |
| R46 | 1, 5.1, 7 Phase 3 | Authenticated REST/OpenAPI daemon and service/worker lifecycle | 20 | NOT_STARTED | — |
| R47 | 1, 5.1, 7 Phase 3 | Daemon-specific gRPC/protobuf and REST parity | 21 | NOT_STARTED | — |
| R48 | 4, 7 Phase 2 | Reference CLI wallet: STK/fallback, bearer transfer and three melt targets | 22 | NOT_STARTED | — |
| R49 | 4, 7 Phase 2 | Reference web UI: one-tap fallback, countdown, melt targets and receipt | 23 | NOT_STARTED | — |
| R50 | 1, 3, 7 Phase 3 | TypeScript, Python, Go, PHP, C# and Rust consumer examples | 24 | NOT_STARTED | — |
| R51 | 5.1, 7 Phase 3 | Actual integration AGENTS.md with published local schema wiring | 24 | NOT_STARTED | — |
| R52 | 7 Phase 2 | Spoof/replay/amount/orphan/503 and crash/race fault suite | 25 | NOT_STARTED | — |
| R53 | 1, 7 Phase 3 | Reproducible CI/packages/container/static-target measurement | 26 | NOT_STARTED | — |
| R54 | 2, 6, 7 Phase 3 | Operator docs, deployment/auth, backup/restore and release preparation | 26 | NOT_STARTED | — |
| R55 | 7 Phase 0, all | Dated live gate evidence and complete traceable final audit | 27,28 | NOT_STARTED | — |


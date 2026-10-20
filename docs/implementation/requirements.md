# PRD requirement traceability

Planning baseline extracted from docs/spec/prd-v1.0.pdf. Stage 01 must check this decomposition for omissions; add rows rather than silently dropping source requirements. Stage 28 audits every row against the source.

All statuses start NOT_STARTED. Statuses: NOT_STARTED, IN_PROGRESS, DONE (implemented with local evidence), BLOCKED, UNSUPPORTED. Local DONE does not imply live verification. External gate evidence is tracked separately in docs/verification-gates.md once created.

| ID | PRD section | Required behavior | Stages | Status | Implementation / test / evidence |
|---|---|---|---|---|---|
| R01 | 1, 5.1 | Single Rust 2024 workspace; core library, processor library/binary, sidecar binary | 00 | NOT_STARTED | — |
| R02 | 1, 2.1 | Dual MIT/Apache-2.0 licensing and self-hosted credential ownership | 00,26 | NOT_STARTED | — |
| R03 | 4, 5.1, 7 Phase 0 | Exact pinned CDK service/protobuf and all seven methods | 00,01,17,18 | NOT_STARTED | — |
| R04 | 4, 7 Phase 0 | NUT-04/NUT-05 custom mpesa and KES/sat compatibility evidence | 01,19 | NOT_STARTED | — |
| R05 | 1, 5.3 | Real CDK proof lock semantics across Pending, errors and recovery | 01,18,19 | NOT_STARTED | — |
| R06 | 5.3, 7 Phase 0 | Independent B2C/B2B idempotency guarantees and gated probing | 01,13,27 | NOT_STARTED | — |
| R07 | 2, 7 Phase 0 | Provider prerequisites and legal claims verified as deployment gates | 01,27 | NOT_STARTED | — |
| R08 | 4, 5 | Explicit state machines and exact amounts/units | 02 | NOT_STARTED | — |
| R09 | 4, 5.2, 5.3 | ACID schema, unique identities and safe state transitions | 02,03 | NOT_STARTED | — |
| R10 | 5.2, 5.3 | Receipt deduplication/allocation, request idempotency and payload identity | 02,03 | NOT_STARTED | — |
| R11 | 5.2, 5.3 | Durable jobs, events, correlation and restart/crash recovery | 02,03,09,12,13 | NOT_STARTED | — |
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


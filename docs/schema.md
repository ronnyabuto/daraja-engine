# SQLite repository

Owner: `daraja-core::repository::Repository`; domain rules remain in `inbound`,
`outbound`, `amount` and `identity`. Fifteen ordered, embedded SQLx migrations live in
`crates/daraja-core/migrations`. SQLx records/checks their versions and checksums;
`build.rs` watches the directory. Never edit an applied migration: add a new one.
Opening a database applies missing migrations and rejects checksum mismatch.
Stage 16 provides PostgreSQL parity; these are SQLite-specific migrations.

## Durability and transaction boundaries

Each of the four pooled connections enables foreign keys, WAL, FULL synchronous
writes and a five-second busy timeout. Pool acquisition is bounded at ten seconds.
`BEGIN IMMEDIATE` obtains SQLite's writer lock before financial reads, avoiding
read-then-write snapshot upgrade races. Every transaction contains only bounded
local database/domain work, with no provider I/O, callbacks or retry sleeps.
Contention/encoding/storage errors return errors without financial success.
Statement logging is disabled; repository errors omit SQL, bindings and file paths.
`close` drains the pool. Durability assumes functioning local storage/fsync; tests
do not simulate power failure, filesystem corruption or distributed filesystems.

Quotes and payouts have monotonically increasing versions. Explicit transitions
require an expected version and update with `WHERE id=? AND version=?`; stale
writers roll back. Confirmation reads the current quote version under the writer
lock, then applies the same conditional update. No public arbitrary state setter
or pool handle is exposed. Terminal decisions and holds use existing domain guards.
A timer or lost lease never fails an ambiguous payout or releases its reservation.

## Tables and additions to the PRD

| Table | Purpose and constraints |
|---|---|
| `inbound_quotes` | Unique quote identity and eight-character account reference, state/version, frozen terms and complete domain snapshot including attempts. |
| `outbound_payouts` | Unique operation and caller request, independently unique nullable provider originator/conversation IDs; immutable canonical payload hash, complete dispatch record and exact wire bytes; state/version, poll/manual deadlines and complete payout snapshot. A trigger prohibits dispatch identity/payload/body mutation. |
| `mpesa_receipts_ledger` | One canonical uppercase ASCII receipt identity and immutable quote allocation, or NULL for exception custody; independent evidence and disposition. Allocation cannot be reassigned through UPDATE. |
| `unreconciled_deposits` | Held/exception receipt custody references the confirmed ledger, with disposition and eventual resolution evidence/time. Underpayment, overpayment, orphan, late, FX-expired and settlement holds remain available for reconciliation. Successful settlement resolves associated holds transactionally. |
| `receipt_observations` | Delivery identity, claimed receipt, timestamp and exact structured observation. Receipt is indexed **without uniqueness**; several observations may claim it. Same delivery ID/data is idempotent; changed data conflicts without reserving a receipt. |
| `stk_attempts` | Separate unique attempt/caller/merchant/checkout identities; quote FK; persisted poll/cooldown times and complete attempt snapshot. Global uniqueness also covers attempts on different quotes. |
| `request_bindings` | Composite caller/operation/key primary key binds canonical SHA-256 payload and original operation. Immutable after insertion. Payout preparation inserts binding atomically with payout/reservation/job; equal retries return original operation, changed payloads conflict. |
| `callback_inbox` | Stage 07 untrusted canonical callback envelopes, static endpoint, nullable quote hint, received time and domain-separated SHA-256 delivery identity. Contains sensitive provider data; never receipt allocation authority. |
| `callback_jobs` | One pending work record per inbox observation, due time and eventual completion time. Inserts atomically with inbox acceptance. Stage 09 mirrors inbound due/completion state from leased durable_jobs; other consumers remain later stages. |
| `durable_jobs` | Unique kind/subject, stable job identity, due time, bounded lease expiry, increasing generation and completion time. Kinds are VERIFY_RECEIPT, POLL_STK, INBOUND_CALLBACK and RECONCILE_PAYOUT. Migration 0004 adds poll_slot, retry_count and static last_reason, preserves leases/generations, and schedules existing inbound callback jobs. |
| `settlement_events` | Stable increasing sequence for consumer cursors plus unique quote identity, time and credit/receipt payload. Quote mutation, all receipt dispositions, hold resolutions and event insertion commit atomically. Stage 09 exposes polling and resumable consumption with consumer-owned durable cursors. |
| `inbound_status_queries` | Stage 09 local query ID, immutable-by-adapter receipt lookup and creation time committed before I/O; nullable separately unique provider originator/conversation IDs. Ambiguous prepared queries are never automatically resubmitted. |
| `balance_reservations` | One exact amount-plus-fee reservation per payout. Release requires a domain-validated terminal evidence resolution and shares its transaction. These are durable commitments, not a substitute for stage 14's balance freshness/admission policy. |
| `audit_records` | Ordered subject/action/time records written with financial mutations. UPDATE/DELETE are rejected. Evidence resides in the associated financial records; no raw callback URL/MSISDN/credentials are copied into audit text. Operator identity/authorization workflow remains stage 15. |

SQL `STRICT` tables and foreign keys enforce identities/custody. Check constraints
cover receipt/reference shape, hash length, nonnegative deadlines and versions,
JSON validity, job kind and reservation unit. Financial `u64` values remain exact
in private JSON snapshots and canonical decimal reservation text; there is no
floating point SQL arithmetic or narrowing to signed monetary integers. Scheduling
timestamps use checked conversion to SQLite's signed integer range; unsupported
values fail with Overflow. The domain continues to check sums, fees and FX using
exact integer arithmetic.

Snapshots store all domain fields, including tariff/FX snapshots, quote/hold
expiry, attempt timing, receipt provenance, payout start/manual times and
correlations. Relational columns enforce uniqueness and support indexed recovery.
Private remote Serde codecs decode persisted snapshots without exposing a public
Deserialize implementation that could bypass guarded constructors. New versions
must migrate both projections and snapshot shape together. The database is trusted
storage; these codecs are not a corruption repair or authentication mechanism.
Wire bytes/snapshots may contain sensitive transport material; provision filesystem
access and protected backup policy before running real transports (stages 04/26).

## Receipt replay and recovery

`observe` commits the observation and its verification job together. A forged first
callback can never occupy the confirmed receipt slot. Repeated observations of the
same receipt with different delivery IDs get independent verification work. Read
`observation(job.subject)` after restart; read `attempt(job.subject)` for STK polls
or `payout(job.subject)` for payout reconciliation. Never treat a claim as evidence.

A worker claims the earliest eligible job in a short write transaction. Leases
last 1–300 seconds. Expired jobs can be reclaimed; each claim increments its
fencing generation. Completion/rescheduling requires that generation and a still
unexpired lease. `finish_job(..., Some(next_due))` reschedules, while None completes.
Workers supply UTC seconds consistently; there is no process-memory clock/queue
required for recovery. The generic completion API remains available. The stage 09 inbound worker shares
lease validation and job completion with confirmation/settlement/event insertion
in one transaction. A crash before commit rolls back; a crash after commit leaves
the durable event available for replay. Stale workers cannot double-settle because
receipt uniqueness, domain terminal guards and transactional event uniqueness
remain authoritative independently of the job lease.

Only a trusted verifier may call `confirm(VerifiedDeposit, ...)`. It checks binding
through the existing accounting aggregate and persists allocation with quote
changes. Optional settlement runs in the same transaction; a failed settlement
rolls confirmation back, so callers handling excess/held funds must use confirmation
without settlement. `settle` later handles verified top-ups/holds with an expected
version and all FX/amount/time checks. A distinct receipt arriving after PAID can
still be confirmed into exception custody without emitting another settlement.
No callback or query success is turned into evidence by this repository. SETTLE
and every other external gate remain disabled.

`prepare_payout` returns only after identity, hash, dispatch/body, reservation,
idempotency binding, audit and reconciliation job have committed. Its successful
return is the explicit crash boundary before transport I/O for stage 12's saga.
A surviving DISPATCHING row is ambiguous even if the process died before dispatch;
recovery reconciles it and keeps CDK Pending. This stage never resubmits. The caller
must enforce the endpoint/environment evidence gates before any later same-identity
probe; a lease or local mock is no such authorization.

## Local evidence

See [SQLite integration tests](../crates/daraja-core/tests/sqlite.rs) for file-backed migration/upgrade, writer,
receipt, rollback, event replay, reservation, lease fencing and abrupt-process
recovery checks. Stage 03 adds persistence primitives; it does not itself implement payout workers, stage 14 liquidity admission or a
public daemon API. Stages 12/13 now supply the
[outbound dispatch](outbound-dispatch.md) and [reconciliation](outbound-reconcile.md)
library services.
Stage 09 now supplies [inbound workers and durable notification replay](inbound-reconcile.md).

## Stage 08 checkout transaction

No migration is needed: the existing quote/reference uniqueness, caller-scoped
request bindings, attempt correlations/timestamps and durable jobs implement
checkout preparation. `prepare_checkout` commits all these records together,
including first-attempt creation; no response or collection I/O precedes commit.
Resends use the reconciled quote version and the same immutable quote identity.
Concurrent stale preparations fail; matching request replays return the stored
quote without dispatching. A collision retry does not leave a request binding or
partial quote. Poll and retry times both derive from the persisted attempt start.
Phone/callback secrets are transient dispatch inputs; the request binding stores
only their behavior-relevant payload hash (the callback URL is configuration, not
a customer input). Recovery never reconstructs or replays an uncertain STK request.
See [checkout contracts](inbound-checkout.md) for failure/recovery and capability limits.

## Stage 10 exception reservations

Migration `0005_deposit_exceptions.sql` adds persisted `deposit_policy`,
receipt-keyed `deposit_risk` and velocity indexing, `deposit_blocks`, idempotent
`deposit_alerts`, immutable gated `refund_requests` and uniquely receipt-keyed
`refund_reservations`. Risk inserts are deferred-FK checked at commit so receipt,
risk, quote, event and worker writes remain one transaction. All refund financial
mutations use the same immediate write transaction as confirmation/settlement.
`REFUND_HELD` closes the quote to further allocations and settlement without
changing immutable receipt allocation or provenance. Refund snapshots retain
reviewed ownership/destination evidence and frozen gross/fee/net values.
[Deposit exception contracts](deposit-exceptions.md) define grace, risk identity,
operator trust boundaries and the execution gate. Existing snapshots without a
payer remain readable with `payer: None`; no earlier migration is rewritten.

## Stage 11 valuation persistence

Migration 0006 adds immutable `payment_quotes` snapshots and `payout_fee_audit`.
Quote snapshots contain normalized destination, principal, tariff effective dates,
version/fee, FX rational/source/expiry, wallet principal/reserve and expiry.
Quoted dispatch records retain the same snapshot; admission compares it against
the durable quote and checks its frozen accounting/expiry. Old dispatch records
with no quote field remain readable. They are not the stage 12 outgoing quote path.

Fee audit rows reference verified PAID payouts and retain nullable independently
established actual fee plus conservative internal total spent. A NULL actual fee
means consume the full reserve. An excess fee is retained with NULL total spent
and raises SettlementHeld. Replays cannot change the audit decision. Update/delete
triggers protect both new tables. No existing migration was edited.

## Stage 12 dispatch admission and observations

Migration 0007 and [outbound dispatch](outbound-dispatch.md) add one payout per
frozen quote, immutable `payout_dispatches` (quote, command, original endpoint),
`payout_dispatch_observations` (transport classification/outcome/time) and
`payout_callback_links` (untrusted inbox candidates by rail and IDs). The claim
transaction includes the existing binding, reservation, audit and reconciliation
job; replay never grants send permission. Acknowledgment IDs, outcome and early
callback correlation share one transaction. Startup recovery marks abandoned
DISPATCHING records unknown without I/O. Legacy duplicate quoted payouts cause
migration to fail closed for review; unquoted legacy fixtures remain supported.

## Stage 13 outbound reconciliation

Migration 0008 adds `payout_reconcile_attempts` (generation, kind, gate reference,
nullable transport outcome and start), `outbound_status_queries` (payout,
TransactionID lookup and immutable acknowledgment IDs), and idempotent
`payout_alerts`. A partial unique index permits one PROBE per payout. Existing
`durable_jobs.poll_slot` counts attempts before I/O; generation/lease checks fence
worker commits. The Paid-receipt unique index also applies to direct repository
resolution; legacy conflicting allocations fail closed at migration.

Manual state, alert and audit are atomic. Final evidence, reservation release,
state, audit and worker completion are atomic. Missing/invalid evidence or a
failed commit retains Pending and its reservation. Manual jobs remain available
for independently verified late evidence, with automated probe/status work stopped.
See [outbound reconciliation](outbound-reconcile.md) for clocks, gates and trust.

## Stage 14 liquidity

Migration 0009 adds `liquidity_policy`, immutable-account/body `balance_queries`,
`quote_float_reservations`, bounded `liquidity_alerts` and immutable `float_sweeps`.
It rebuilds payout admission and its observation/link children together, preserving
existing rows and foreign keys; only internal sweep admission may have a NULL
quote and the exact sweep command. Customer quoted admission keeps its unique
quote binding. The migration does not manufacture balance evidence or holds for
historical unconsumed quotes. Existing pending liabilities remain counted.

Quote save/check/reservation is one writer transaction. Dispatch converts that
hold and creates the existing payout reservation in one transaction; failure rolls
back both. Only undispatched holds expire. Paid release times serve as conservative
snapshot debit watermarks, and unresolved payouts never age out of accounting.
Latest verified query sequence prevents out-of-order callbacks from replacing
newer observations. Sweep admission includes snapshot uniqueness, unresolved
exclusion, cooldown and gross daily limits under that same writer lock.
See [liquidity](liquidity.md) for trust, freshness and account separation.

## Operator recovery additions (stage 15 in progress)

Migration 0010 adds append-only `operator_commands`, keyed by actor and command
ID, with full reviewed command/evidence, reason, expected payout version, request
hash, saved result/version and timestamp. Manual resolution commits this record,
the payout CAS, exact reservation release and job fencing together. Conflicting
terminal evidence is retained without changing financial state. Access and
recovery instructions are in the [operator runbook](operator-runbook.md).

Migration 0011 adds `operator_alert_deliveries`. Insert/change triggers enqueue
manual and liquidity alerts transactionally, including liquidity recovery, and
backfill existing alerts. Delivery has bounded generation leases, persisted
attempts/backoff and acknowledgment evidence; a restart cannot lose an unacked
event. Queue pages omit raw customer/receipt data. Refund execution and FX
exception allocation have not changed in these migrations and remain pending.

Migrations 0012 and 0013 add immutable actor-scoped hold-review and inbound
verification-retry commands. Audit insertion and the guarded change are atomic.

Migration 0014 binds refund reservations to unique outgoing quotes, with an
immutable actor/case authorization. Verified Paid reservation release closes
associated exception custody transactionally. Migration 0015 links original and
replacement FX quotes to an immutable owner/reason/evidence request and saved
settlement event. Original receipt allocation remains immutable; disposition
becomes Settled and the old quote cannot mint. Refund reservations and settled
receipt checks enforce exclusive consumption under the existing writer lock.

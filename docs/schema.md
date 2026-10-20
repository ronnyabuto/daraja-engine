# SQLite repository (stage 03)

Owner: `daraja-core::repository::Repository`; domain rules remain in `inbound`,
`outbound`, `amount` and `identity`. Two ordered, embedded SQLx migrations live in
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
| `durable_jobs` | Unique kind/subject, stable job identity, due time, bounded lease expiry, increasing generation and completion time. Kinds are VERIFY_RECEIPT, POLL_STK and RECONCILE_PAYOUT. |
| `settlement_events` | Stable increasing sequence for consumer cursors plus unique quote identity, time and credit/receipt payload. Quote mutation, all receipt dispositions, hold resolutions and event insertion commit atomically. Consumer cursors belong to future adapters. |
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
required for recovery. Completion need not share settlement's transaction: a crash
between them causes verification redelivery, and the confirmed ledger must be
checked before acknowledging that work. Stale workers cannot double-settle because
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

See [03-sqlite.md](implementation/evidence/03-sqlite.md) and
`crates/daraja-core/tests/sqlite.rs` for file-backed migration/upgrade, writer,
receipt, rollback, event replay, reservation, lease fencing and abrupt-process
recovery checks. Stage 03 adds persistence primitives; it does not implement the
stage 09/12/13 provider workers, stage 14 liquidity admission or a public daemon API.

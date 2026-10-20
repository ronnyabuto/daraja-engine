# Financial domain contracts (stage 02)

Owners: `daraja-core::{amount,identity,inbound,outbound}`. These contracts were
checked against the source specification; internal PRD inputs are maintained
locally and are not distributed with the source tree.
[Pinned protocol findings](protocol-contract.md) override unsupported provider
claims: no callback alone settles, no duplicate error proves payment, and no
timer proves nonexecution. Stage 01 local acceptance is DONE. All external
[capability gates](verification-gates.json) remain disabled.

Stage 03 now implements the persistence follow-through described below; see
[schema and recovery contracts](schema.md) and
[SQLite boundary tests](../crates/daraja-core/tests/sqlite.rs). References below to
future stage 03 work describe the original stage 02 handoff.

## Boundary and evidence

This stage implements pure domain transitions, not transport, persistence, jobs
or CDK service adapters. `Accounting` is a volatile aggregate. Its exclusive
mutable access serializes local transitions and its settlement method changes
quote, receipts and event together after validation. It is not a database or a
durability claim. Stage 03 must enforce the same contract with database uniqueness,
compare-and-set and a single transaction, including durable outbox insertion.
Network I/O must remain outside that transaction. Process recovery, cross-process
writers and dispatch-after-commit evidence remain stage 03 and later work.

`ReceiptObservation` claims are retained separately and never allocate a receipt,
credit a quote or authorize settlement. Repeated observations may claim the same
receipt. `VerifiedDeposit::attest` and `Payout::resolve` are explicit trusted-caller
boundaries: their evidence must already have been independently verified by a
future verifier. An `EvidenceSource` references an authenticated provider record
or an operator-reviewed statement. Naming a source does not authenticate it.
These constructors check shape and binding; they do not contact a provider or
turn a callback into trusted evidence. They must not be exposed directly as
unauthenticated request DTOs. Synthetic attestations in tests establish no live
guarantees; SETTLE remains disabled.

Deposit evidence must attest final receipt, exact KES amount, collection shortcode,
quote/rail binding, receipt time and provenance together. A successful query which
omits receipt or amount cannot supply that attestation. STK binds an existing
CheckoutRequestID; C2B binds the quote's eight-character account reference. A
missing quote is an error when evidence claims a quote; independently verified
unmatched funds use `quote_id: None` and remain orphaned.

Payout final evidence binds operation ID, canonical payload hash, exact amount
and destination. PAID requires the actual provider receipt. FAILED requires
independent final nonexecution evidence. Timeout, absence, duplicate rejection,
HTTP errors and manual escalation do not meet that standard. Audited evidence
acquisition and operator authorization are later stages, not supplied by enums.

## Exact accounting and immutable identity

- `Amount` is a checked `u64` with `KesMinor` (1/100 KES) or `Sat`. Add/subtract
  reject currency mismatch, overflow and underflow. Whole-KES provider conversion
  rejects fractional KES instead of rounding. No floating point is used.
- Quote terms freeze expected KES, credited amount, tariff version/fee, quote
  expiry, optional rational FX snapshot/source/expiry and settlement hold deadline.
  KES credit is exactly expected minus fee. Sat credit is at most floor(net × FX
  numerator / denominator), allowing requested sat amounts with collection dust.
  Products use `u128`, results must fit `u64`, zero quotes and zero credit are rejected. Rounding dust remains unissued; fees cannot be added
  to credit. Stage 11 supplies [validated tariffs and frozen FX](tariffs-fx.md);
  liquidity admission remains stage 14. No production tariff/rate defaults exist.
- Quote expiry concerns **receipt time**: receipt time at/after expiry is held.
  An independently verified on-time KES receipt may settle after quote expiry.
  There is no automatic grace for late deposits. FX validity is stricter:
  verification and settlement must both precede FX expiry. A hold never extends
  FX validity. Expired FX requires later refund/re-quote handling, never stale issuance.
- Provider attempt start, poll deadline and resend cooldown are distinct from
  quote expiry, FX expiry and settlement hold. Time arguments are explicit UTC
  seconds. Deadline equality means expired/eligible, as appropriate. A timer
  alone cannot change value, declare payment or release proofs.
- `RequestKey` scopes caller key by caller and operation. `RequestRegistry::bind`
  returns the originally bound operation on a replay, even if a retry supplies
  a fresh proposed ID. A different canonical payload returns IdempotencyConflict.
  Persistence of the binding is required in stage 03.
- Canonical request v1 hashes SHA-256 over `daraja.request.v1\0`, then operation,
  quote ID, destination, tariff version, unit (`kes_minor` or `sat`), amount and
  options. Strings are UTF-8 with `u64` big-endian byte lengths; amount and option
  count are `u64` big-endian; option key/value pairs are in BTreeMap order, each
  length-prefixed. Options are exact strings, not implicitly normalized JSON.
  Adapters must include every behavior-affecting input in these fields/options.
- `DispatchRecord` freezes that payload/hash, operation ID, exact wire body,
  tariff and amount-plus-fee reservation. It validates reservation accounting,
  not live liquidity. No transition changes them. Stage 12 must commit them
  before I/O; only independently gated retries may reuse the same identity/body.
- Caller request, caller OriginatorConversationID, proxy OriginatorConversationID
  and provider ConversationID remain separate. Conflicting acknowledgments fail
  without mutation. STK instead records its caller request, MerchantRequestID
  and CheckoutRequestID; it does not invent a payout OriginatorConversationID.
- Confirmed receipt keys have one uppercase ASCII alphanumeric representation;
  invalid spellings fail closed. Observations and confirmed allocations have
  separate storage. One actual receipt can occupy one confirmed slot and one
  accounting allocation, including exception custody. Debug output redacts IDs,
  canonical request options and frozen wire bodies.

## Inbound quote transitions

Names below correspond to Rust enum variants using conventional persisted names.
No public setter permits arbitrary state changes.

| From | Trigger/evidence | To |
|---|---|---|
| ISSUED | Start persisted STK attempt with unexpired quote/FX | ISSUED; attempt DISPATCHING |
| ISSUED / STK_SENT | Matching immutable merchant/checkout acknowledgment | STK_SENT |
| ISSUED / STK_SENT | Additional attempt after previous 35-second cooldown, before expiry | Same quote state; independent attempt |
| Any unpaid quote | Separately verified, on-time, bound receipt; sum below expected | UNDERPAID_HELD |
| Any unpaid quote | Same evidence; sum equals expected; hold not elapsed | SETTLEMENT_HELD |
| Any unpaid quote | Same evidence; sum equals expected; hold elapsed | VERIFYING (verified funds ready) |
| Any unpaid quote | Same evidence; sum exceeds expected | OVERPAID_HELD |
| Unpaid, exactly funded | Atomic settlement: all receipts independently verified, FX valid, hold elapsed, settlement time ≥ verification times | PAID plus one settlement event |
| PAID | Distinct additional verified receipt from either rail | PAID; extra receipt OVERPAID_HELD in exception custody |
| PAID | Repeat settlement / new STK / acknowledgment | Rejected; no terminal regression |
| Any | Untrusted observation, passage of quote TTL, STK error | No financial state change |

Receipt amount summation is checked before any mutation. A top-up uses a distinct
receipt. UNDERPAID_HELD can become exactly funded via another receipt. OVERPAID_HELD
does not automatically mint the expected portion; it requires stage 10 exception
resolution. Exact funds held for settlement can settle when the hold elapses,
but that call still validates all value and FX predicates.

STK attempt transitions are DISPATCHING → SENT on acknowledgment; DISPATCHING or
SENT → UNKNOWN on transport ambiguity; UNKNOWN → SENT on matching acknowledgment;
any nonterminal attempt → COMPLETED/FAILED on an attempt-level result observation.
Repeated UNKNOWN and identical acknowledgments are safe. COMPLETED/FAILED cannot
regress. A successful attempt observation is not independently verified value.
One failed STK attempt cannot fail the quote or disable its C2B rail. All attempts
remain associated with the same quote, including older checkout IDs.

## Deposit custody transitions

| Input / prior state | Evidence and accounting result | Receipt state |
|---|---|---|
| Untrusted observation | No allocation; verification may be retried independently | Observation only |
| New verified receipt, unmatched | Confirmed exception allocation; never mint | ORPHAN_UNCLAIMED |
| New verified receipt, received at/after quote expiry | Confirmed exception allocation | LATE_HELD |
| New verified receipt, FX expired at verification | Confirmed exception allocation | FX_EXPIRED_HELD |
| New bound verified receipt | Single quote allocation, total below/above expected | UNDERPAID_HELD / OVERPAID_HELD |
| New bound verified receipt, exact total | Single quote allocation | VERIFIED / SETTLEMENT_HELD |
| Any quote-allocated receipt | All-or-nothing quote settlement, event and all associated receipt updates | SETTLED |
| SETTLED | Reuse receipt on same/another quote or rail | Rejected |
| Any confirmed receipt | Duplicate receipt delivery | Rejected without replacing provenance/allocation |

Held records retain their verified evidence and allocation. Earlier top-up
records keep their held disposition until the common settlement marks all
quote allocations SETTLED. A new receipt after PAID is retained as an exception,
never another quote allocation. If both full-value rails confirm before settlement,
the combined overpayment is held; if one settles first, the second is held
separately. Neither ordering can emit two credits. Refund authorization, refunds
net of fee, re-quoting and splitting excess are deliberately stage 10/15 work;
there is no unsafe generic state setter or provisional refund success here.

## Outbound transitions and CDK projection

| From | Evidence/trigger | To | CDK state |
|---|---|---|---|
| New | Immutable validated dispatch record (must be committed before I/O) | DISPATCHING | Pending |
| DISPATCHING | Transport error, 500/503, timeout, QueueTimeOutURL, worker recovery | UNKNOWN_IN_FLIGHT | Pending |
| DISPATCHING / UNKNOWN_IN_FLIGHT / PENDING | Matching acknowledgment | PENDING | Pending |
| DISPATCHING / UNKNOWN_IN_FLIGHT / PENDING | Duplicate rejection observation | PENDING | Pending |
| UNKNOWN_IN_FLIGHT / PENDING | More transport ambiguity | Same state | Pending |
| Any nonterminal | Manual deadline elapsed | MANUAL_RECONCILIATION_REQUIRED | Pending |
| MANUAL_RECONCILIATION_REQUIRED | More ambiguity, duplicates or matching acknowledgment | Same state | Pending |
| Any nonterminal, including manual | Independently verified, dispatch-bound paid evidence with receipt | PAID | Paid |
| Any nonterminal, including manual | Independently verified, dispatch-bound definitive nonexecution | FAILED | Failed |
| PAID / FAILED | Any further transition, including conflicting final evidence | Rejected | Unchanged |

There is no `Unpaid` projection for possibly dispatched payouts. The local CDK
projection is a domain contract, not the service adapter (stage 18). Manual
escalation never releases ecash proofs. No probe/resubmission method is enabled
by this stage; DUP-B2C and DUP-B2B remain independent external gates.

## Local acceptance and follow-through

`crates/daraja-core/tests/domain.rs` exercises legal paths and terminal rejection,
44,550 small-domain fee/FX combinations (including rejected zero-credit cases),
checked overflow/currency boundaries, ambiguous recovery/manual Pending,
evidence binding, idempotent replay/conflict, receipt spelling/duplicate handling,
spoof-first recovery, top-ups, both rail orderings, a two-thread serialized rail
race, expiry/hold boundaries and failed-operation rollback. The threaded test
proves the aggregate contract under a mutex, not SQL isolation or durability.

Stage 03 must reproduce the invariants on file-backed SQLite with rollback,
competing writers and reopen/crash tests. Stages 09/13/15 provide trusted evidence
acquisition and operator resolution; stages 17–19 verify actual CDK behavior.
Run `cargo test -p daraja-core --test domain --locked` to reproduce the domain checks.

## Stage 08 checkout service

[Durable checkout](inbound-checkout.md) now implements quote creation and explicit
resends over these transitions. The 35-second timer is only an eligibility check: a
new prompt also requires prior-attempt reconciliation, an active unexpired quote
and a successful version-checked preparation commit. Failure query-code mappings
are disabled by default pending environment-specific operator verification. A
failed callback alone cannot authorize resend; known correlations are queried
again. A missing checkout ID after possible dispatch blocks resend. Immediate
manual fallback has no payer-phone equality requirement. The separate dispatch
metadata recording path can bind a late acknowledgment after PAID; it preserves
financial state and terminal attempt results so additional payments can still be
matched and held as excess. The original public acknowledgment transition still
rejects PAID.

## Stage 09 verification worker

[Inbound reconciliation](inbound-reconcile.md) now schedules provider observations
and applies these domain predicates under a fenced SQLite transaction. A separate
trusted `DepositVerifier` must establish complete independent value evidence; the
default `UnavailableVerifier` leaves it unresolved. Success codes alone never
construct trusted value. Already verified settlement holds can release from
durable evidence, while rechecking FX, amount and time. Quote settlement, event
insertion and worker completion commit together; consumption replays by sequence.

## Stage 10 exception policy

[Deposit exceptions](deposit-exceptions.md) now implement the previously deferred
local exception/refund boundary. `UNDERPAID_HELD` / `OVERPAID_HELD` can transition
to `REFUND_HELD` only with an atomic full-balance refund reservation; this state
cannot mint, accept further quote allocations or admit validation top-ups.
Additional verified receipts remain separate exceptions. KES receipt-time grace
is explicitly configured and defaults to zero; sat/FX expiry remains strict.
Persisted timed risk holds and manual velocity/reversal blocks apply through all
repository settlement paths. Operator block release does not bypass amount, FX,
verification or timed hold checks. Refund requests remain `EXECUTION_GATED`;
operator authentication/workflows and outbound execution remain later stages.

## Stage 11 valuation

Outgoing `PaymentQuote` freezes destination, principal, tariff effective dates,
version/fee, rational FX, wallet reserve and expiration. Repository snapshots and
fee audit decisions are immutable; new quoted dispatches validate expiry before
admission. Existing pending dispatches retain their terms after expiry.
Actual-fee evidence cannot refresh rates; excess fees remain audit exceptions.
See [tariffs and FX](tariffs-fx.md) for exact rounding and CDK unit mapping.

## Stage 12 dispatch orchestration

[OutboundPayments](outbound-dispatch.md) now commits and claims the immutable
quoted operation before one transport submission. Same-key payload replay returns
the original operation; quote uniqueness prevents another key spending it again.
QueueTimeOutURL can move acknowledged Pending to UnknownInFlight. Both project
to CDK Pending; terminal evidence and manual states are preserved. Recovery of
DISPATCHING never grants another send. Early callbacks remain untrusted inbox
observations and are correlated with acknowledgment IDs transactionally.

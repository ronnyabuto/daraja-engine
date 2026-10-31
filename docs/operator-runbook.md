# Operator recovery

The `daraja-core::operations::OperatorService` library supports authenticated
redacted payout views, bounded pending/exception queues, safe verification retry,
hold review, refund authorization/execution, held FX re-quoting, manual payout
resolution and leased alerts. Deployment wiring supplies the trusted repository,
provider clients, FX oracle and reviewed ownership evidence. There is no network
listener or CLI; daemon mounting remains a later stage.

Deployment configuration owns a separate credential list (maximum 64 actors).
Tokens must contain at least 32 bytes, be generated with suitable randomness,
and travel only over a protected operator channel. The service retains SHA-256
digests, compares them in constant time and assigns either Observer or
Reconciler permissions. An empty credential list denies everyone. Application
credentials must never be copied into this list. Database/repository access
remains a trusted local integration boundary; possession of the SQLite file
or trusted Rust library access is outside service authentication.

## Manual payout review

1. Use `payout` to read the current version and redacted state. Manual escalation
   maps to CDK Pending. Keep all existing proofs and reservations held.
2. Obtain independently authenticated transaction records or a reviewed portal
   statement for the exact immutable operation, payload hash, amount and
   destination. A callback, timeout or absence in a portal is insufficient.
3. Submit `resolve_payout` with the current version, a stable command ID, an
   opaque case/reason reference and the evidence reference. Paid requires the
   actual receipt; Failed requires definitive non-execution evidence referencing
   the same reviewed record. The service validates binding; the operator is
   responsible for reviewing the independent evidence's authenticity/finality.
4. On a lost response, replay the identical command as the same actor. It returns
   the saved result even after restart. Changed commands sharing an ID conflict.
   A new command with a stale version conflicts: review the current record.
5. Contradictory terminal evidence records a durable `CONTRADICTORY` command and
   returns `ContradictoryEvidence`; it never rewrites a Paid/Failed decision.
   Escalate the incident for independent review. Do not send another payout or
   assume this recovers proofs already released by an earlier Failed decision.

The command, actor, reason, evidence, expected version, result and timestamp are
append-only. The command record, financial change, exact reservation release
and reconciliation worker fencing commit together. Successful manual resolution
updates the existing CDK state projection; actual CDK service wiring remains
stages 18–19. No live provider capability is enabled by these commands.

## Synthetic local manual escalation drill

Run from the repository root; fixtures use only a temporary file-backed SQLite
database and synthetic evidence. No provider request is issued.

```sh
cargo test -p daraja-core --test operations manual_resolution_drill_replays_after_operator_restart --locked --offline -- --exact --nocapture
cargo test -p daraja-core --test operations --locked --offline
```

Expected: the first command passes one test. It creates a durable payout,
escalates it to manual and asserts CDK Pending; rejects a wrong amount and
asserts it is still Pending; accepts exact reviewed Paid evidence with receipt
`ACTUAL17`; closes/reopens SQLite and replays the command with the same result.
It verifies the complete immutable audit record and one released reservation.
The second command currently passes nine tests, additionally checking access
control/redaction, concurrent replay/stale commands, late contradictory success
and rollback/recovery after an audit insert failure, bounded queues and alert
retry/recovery.

## Queue and alert delivery integration

`queue` accepts `PendingPayouts` or `ExceptionDeposits`, an exclusive row cursor
and a limit from 1 to 100. Pages contain amount, state and a hashed subject;
they omit raw receipt identities, destinations and wire bodies. Queue membership
is live, so restart scans at cursor zero to find older records that become
eligible again. These scans are operational projections, not fund allocations.

Manual payout escalation and liquidity state changes enqueue alert deliveries
in the same SQLite transaction through migration 0011 triggers. Existing alerts
are backfilled. Identical liquidity refreshes do not create new deliveries;
recovery creates a distinct event. `claim_alert` returns one due event under a
30-second generation lease. A Reconciler-owned sink integration sends it outside
SQLite, deduplicates by sequence, and calls `finish_alert` with a durable sink
acceptance reference. A lost acknowledgment may repeat delivery. Sinks should
also apply sequence ordering per subject to avoid presenting delayed older
alerts as current status.

On sink failure, finish with no evidence: attempts/backoff persist, from 10
seconds to a maximum of 300 seconds. On process exit, the lease expires and
another operator process may claim the same sequence with a new generation.
Expired generations cannot acknowledge; an identical successful acknowledgment
can be repeated after a lost response. Claims, retries and delivery evidence
are audited. Alert delivery never resolves money or releases proofs. No external
sink is contacted by the library or the synthetic tests.

## Safe inbound verification retry

A Reconciler may submit `retry_verification` with a stable command ID, job ID,
expected generation, opaque reason and evidence reference. Only due, unfinished
POLL_STK, VERIFY_RECEIPT or INBOUND_CALLBACK work without a live lease is eligible.
Backoff deadlines, poll slots and retry counts are preserved. The command fences
older workers and leaves verification to the existing leased inbound reconciler;
it supplies no settlement evidence or provider permission. Identical replay by
the same actor survives restart; changed terms and stale generations conflict.
The command and audit commit together. This command does not retry payouts.

`review_deposit_hold` requires Reconciler credentials and an expected block reason.
Its immutable command and audit commit with block removal. Existing value, time,
FX and evidence predicates still apply. Replaying a prior command cannot remove
a subsequently inserted block. Reviewers must obtain fresh evidence for new blocks.

## Refund review and execution

Use `request_refund` as a Reconciler after independently reviewing ownership and
the original payer destination for every receipt (at most 100 per service call).
The service derives the actor from authentication. Receipt numbers and callback
phone fields do not establish ownership. The reservation freezes gross, net and
B2C tariff; all quote allocations must be included. It prevents competing mint,
top-up allocation and refunds before any network request.

Construct a current KES `PaymentQuote` from the deployed B2C tariff schedule, for
exactly the reserved net and reviewed canonical phone destination. Its fee and
version must equal the reservation and its total must equal gross. Submit
`execute_refund` with the reservation ID, case reference and quote. Use the same
repository for OperatorService and OutboundPayments. The binding, float hold and
authorization audit commit atomically before the existing one-send payout saga.
Ordinary `make_payment` rejects refund quotes. Repeated calls and restarted
operators use the same operation; ambiguous transport results remain Pending.

Only independent Paid evidence closes exception custody, transactionally with
payout resolution and balance-reservation release. Receipt reservations remain
permanent, preventing reuse after payment. Definitive nonexecution releases only
outbound float and leaves deposit custody held for review. There is no automatic
resend, tariff substitution or cancellation/reallocation. An expired undispatched
refund quote fails closed and requires further recovery implementation; do not
try another reservation against the same receipts.

## Held FX re-quote

`requote_held_fx` requires a Reconciler, the original quote/version, new quote ID
and eight-character account reference, reviewed owner, case and evidence references.
It fetches the configured oracle before opening the writer transaction. Current
FX must expire within 300 seconds. The service credits the entire verified held
balance less the original collection tariff at the fresh exact rate. Timed holds,
manual blocks, settled receipts and refund reservations prevent the operation.

The transaction retires the old quote as RefundHeld, creates a Paid replacement
and one settlement event, marks the original receipt records Settled, resolves
exception custody and writes the immutable command/audit. Original provider
provenance and ledger allocation stay intact; the event and requote record link
receipts and old/new quote ownership. Consumer wiring must deliver the new event
to the reviewed owner; never treat the old quote as paid. Identical replay returns
the saved event even if the oracle is unavailable after restart. New top-ups to
the retired quote enter separate exception custody.

## Late independent payout evidence

After a late callback for terminal money, the trusted supervisor may call
`OutboundReconciler::inspect_late_evidence`. It consults the independent verifier
without probing or sending. Bound contradictory final evidence is saved through
the same immutable command boundary and enqueues a durable operator alert. Paid
or Failed money is never rewritten. The default verifier remains unavailable.

## Additional synthetic drills

```sh
cargo test -p daraja-core --test collection operator_refund_uses_one_send_saga_and_consumes_exception_only_after_paid --locked -- --nocapture
cargo test -p daraja-core --test exceptions authenticated_fx_requote_consumes_held_funds_once_with_fresh_credit --locked -- --exact
cargo test -p daraja-core --test exceptions fx_requote_audit_rollback_and_refund_race_preserve_single_owner --locked -- --exact
cargo test -p daraja-core --test collection independent_late_contradiction --locked
```

Expected: refund 503 and acknowledgment cases each send once, remain Pending,
replay through a fresh operator/repository and close custody only after verified
Paid evidence. FX produces one fresh-rate event, rejects refund reuse and replays
after reopen; failed audit rolls back and the refund/requote race has one winner.
Late contradictory evidence creates one durable incident/alert and leaves Failed
unchanged. All fixtures are synthetic and use temporary SQLite files and loopback
HTTP; none calls a provider or moves money.

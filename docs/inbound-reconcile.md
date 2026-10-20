# Inbound reconciliation and durable notifications

Owners: `daraja-core::reconcile` coordinates provider work; `repository` owns
leases, receipt allocation and settlement transactions. Domain rules remain in
`inbound`. This is a library boundary; daemon supervision and the pinned CDK
adapter remain stages 20 and 17 respectively.

## Evidence and four gates

1. The [ingress router](webhook-ingress.md) admits callbacks using the configured
   secret and stored quote/checkout binding. Workers reload the admitted envelope
   from SQLite and check shortcode, checkout/merchant or account reference again.
   Missing callbacks can instead be recovered from a persisted STK attempt.
2. Untrusted observations never reserve a receipt. The canonical receipt ledger
   has global uniqueness across quotes and rails. Allocation happens only after
   independent verification. Duplicate verified delivery completes its job without
   creating a second allocation or settlement.
3. Independent evidence must agree with every supplied receipt, exact KES amount,
   receipt time, shortcode, quote and rail binding. Provider dates are parsed as
   EAT and converted to UTC seconds. The domain applies receipt-time quote expiry,
   current FX expiry, settlement holds and checked amount sums. Independently
   confirmed underpayments, excess, late deposits and orphans retain existing held
   custody; their resolution remains stage 10. An on-time KES receipt may settle
   after quote expiry. No timer is financial evidence.
4. Every amount requires a successful bound query **and** a `DepositVerifier`
   attestation of independently established final receipt, amount, destination,
   quote/rail and receipt time. `UnavailableVerifier` always leaves this gate
   unresolved. There is no built-in live evidence source. Implementing this trusted
   interface requires endpoint/environment evidence and approved provenance; copying
   callback fields or returning an attestation because ResultCode is zero violates
   its contract. Synthetic test records prove the local boundary only.

The [protocol evidence limits](protocol-contract.md#settlement-trust-decision)
remain in force. STK query success does not independently establish receipt or
amount. TransactionStatus results arrive over unsigned callbacks; correlation,
static callback secrets, validation prechecks and OAuth request acceptance do not
establish independent authenticity. Missing fields are never filled from an
untrusted callback and then called independently proven. SETTLE and all external
capability flags remain disabled.

## Durable scheduling and query flow

`InboundReconciler::run_once` claims at most one inbound job with a 30-second lease.
It handles POLL_STK, INBOUND_CALLBACK and VERIFY_RECEIPT without claiming payout
work. Provider query plus evidence acquisition is bounded to 20 seconds. Network
I/O happens outside SQLite transactions. Completion, retry and financial commits
check generation and unexpired lease; a stale worker cannot commit financial work.

A persisted STK attempt starts with a due job at start +35 seconds. Poll progress
then schedules start +65 and +110, using absolute times rather than adding delays
to the last poll. Restart resumes due work. If a deadline has already passed, the
next catch-up poll is at least one second later. Spike arrest retries persist
2, 4, 8, 16 and 30 second delays, at most five per poll slot; exhaustion advances
the slot. After the initial slots, unresolved work retries every 300 seconds.
Other unresolved callback/receipt work also retries after 300 seconds. Unknown
codes, timeouts and absent evidence never fail a quote or automatically resend STK.
An attempt without provider checkout identity remains unresolved.

`ProviderQueries` performs one STK query per scheduled execution and checks both
checkout and merchant correlation. It does not use transport-level retry sleeps;
the database owns the retry deadline. Callback-driven queries are independent of
the missing-callback poll schedule.

C2B uses its own TransactionStatus request, with TransactionID equal to the claimed
receipt and the collection account/credentials. The local query ID and receipt
are committed in `inbound_status_queries` before I/O. Provider originator and
conversation IDs are separately bound after acknowledgment. Matching early
results are retained in the inbox and become visible once both IDs are known.
Wrong correlation, receipt, supplied amount or unknown result code remains
unresolved. A timeout is not a result. A prepared status request is never
resubmitted automatically: a crash before acknowledgment or transport ambiguity
can require operator/provider reconciliation, even if no request actually reached
the provider. Reopening still polls the existing correlation; it never invents a
fresh query identity. These conservative limits do not claim provider idempotency.

Confirmation, optional settlement, event insertion and worker completion share
one write transaction. A failed event insertion rolls all of them back. Exact
verified funds held until a deadline keep their job scheduled at that deadline;
release uses already durable evidence and rechecks current amount/FX/time guards.
A second provider call is unnecessary for this hold release. Process death after
commit leaves the event available for delivery. Receipt uniqueness and the unique
quote event protect concurrent callback and poll workers independently of leases.

## Cursor and replay contract

`Repository::events_after(sequence, limit)` returns increasing durable sequences,
exclusive of the supplied cursor, in batches capped at 1000. Zero replays all
history. Events have a stable quote identity and exact credited amount; the event
is an accounting outcome, never a callback projection. Events are retained.

`IncomingEvents::resume(repository, cursor)` offers the same history as a polling
stream. `next()` waits using 250 ms database polls when empty. Cancellation or
disconnect loses no event. The offered event repeats until `acknowledge(sequence)`
advances the local cursor; acknowledgment accepts only the offered sequence.
Consumers persist their handled cursor with their own effects before advancing
or reconnecting. Local acknowledgment is not a durable consumer checkpoint.
Reconnect from the last durable cursor; an interrupted acknowledgment can redeliver
an event. Consumers must deduplicate by sequence/quote. This is at-least-once
delivery over exactly one accounting settlement, with SQLite as the source of
truth. No memory broadcast, pruning or CDK cursor mapping is implemented here.

## Local verification

`cargo test -p daraja-core --test reconcile --locked` exercises real loopback HTTP
and file-backed SQLite, fake-clock deadlines/backoff, forged observations and
field mismatches, early status results, timeout/unknown recovery, migration,
competing and stale workers, held release, transactional rollback, a child process
exiting after settlement commit, and replay after reconnect. Independent records
are explicitly synthetic; no live money movement or provider guarantee is tested.

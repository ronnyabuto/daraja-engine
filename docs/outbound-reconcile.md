# Outbound reconciliation

`outbound::OutboundReconciler` is a library worker over file-backed SQLite.
It claims only `RECONCILE_PAYOUT` jobs with a 30-second durable generation lease.
A claim grants reconciliation work; initial dispatch still belongs exclusively
to [OutboundPayments](outbound-dispatch.md). Supervisors call `run_once`, sleep
when idle, and bound concurrency. Service lifecycle wiring remains later work.

## Bounded automation and manual recovery

`ReconcileConfig` defaults to a 60-second poll interval, 7,200 seconds of automated
polling, 120 attempts and 43,200 seconds `max_pending_ttl_seconds` (MAX_PENDING_TTL).
Intervals and budgets must be positive; the hard TTL cannot precede the automated
time budget. All ages derive from the original durable payout start. The frozen
`Payout::manual_after` is an additional upper bound, so a configuration reload
cannot extend that deadline. Attempt counts are committed before I/O and survive
cancellation, process exit and configuration reload. A due deadline preempts even
a job scheduled beyond it; an active lease must expire before another worker acts.
An unavailable supervisor/database can delay escalation, never manufacture finality.

Automated budget exhaustion, hard TTL, contradictory observations, or the absence
of a usable query ID and safe probe cause `MANUAL_RECONCILIATION_REQUIRED`. The
state change, one durable `payout_alerts` row and audit record commit together.
`Repository::payout_alerts` exposes the durable local operator queue. External
alert delivery and authenticated operator workflows remain stage 15. A manual
job checks independently available late evidence every 60 seconds without issuing
further payout probes or TransactionStatus requests. More acknowledgments,
timeouts and duplicates cannot undo manual escalation. The original alert is
retained after resolution.

Every ambiguous/manual state maps to CDK Pending and keeps its reservation.
Elapsed time, a missing portal record, worker/storage errors, shutdown and query
timeouts never authorize Failed. This is the core projection contract; actual
mint proof-state integration remains stages 18–19.

## Stage 1: separately approved duplicate probe

Probe evidence is empty by default. Each `ProbeEvidence` is an operator-supplied
reference to independently reviewed protocol **and sandbox** evidence for the exact
endpoint/environment and command, with an expiry and maximum operation age covered
by the verified duplicate-retention guarantee. Evidence must establish account/app
scope, original-ID handling, payload equality and retention sufficient for the
request delivery window. Documentation samples and local fixtures do not establish
these guarantees. B2C `BusinessPayment`, B2B `BusinessBuyGoods` and B2B
`BusinessPayBill` approvals do not authorize one another. Production and sandbox
URLs differ; a loopback approval is useful only for synthetic tests.

Only an unacknowledged payout without a concrete TransactionID, prior duplicate
observation or consumed probe can probe. Admission commits the gate reference and
consumes the **single** probe allowance before I/O. A crash before send therefore
consumes that allowance too; recovery escalates if no queryable evidence becomes
available. There is no repeated probe loop or fresh-ID fallback.

The adapter sends the original stored wire bytes, original OriginatorConversationID,
command, encrypted credential, destination, amount and callback URLs. It checks the
stored endpoint against the client's exact environment/endpoint, command and
remitter shortcode. After OAuth and immediately before HTTP, it rechecks the lease,
prepared attempt, nonterminal state, gate expiry/retention and automation deadline.
Transport does not replay the submission, including after a 401 refresh. As with
any network timeout, expiry/cancellation cannot recall a request already sent;
the deployed provider guarantee must cover delivery uncertainty.

`500.002.1001` is prior-registration/in-flight evidence only. It remains Pending,
never Paid, and consumes the probe. A valid synchronous acknowledgment can bind
proxy IDs and TransactionID atomically with early payout callback correlation.

## Stage 2: TransactionStatus observations and independent final evidence

The concrete adapter uses only `TransactionID` lookup, via the existing typed
TransactionStatus client. Candidates come from stored synchronous observations or
rail/ID-correlated payout callbacks. Candidates are hints, never settlement proof.
Caller OriginatorConversationID, proxy originator and ConversationID are never
substituted for a TransactionID. Conversation lookup remains unsupported by this
worker even if a transport client has separately enabled STATUS-ID. An
acknowledgment without a usable ID can therefore require immediate manual review.

Each query has a durable payout/generation-specific identity and exact lookup
prepared before I/O. Its acknowledgment binds both provider IDs without overwrites.
Early query callbacks remain in the authenticated ingress inbox and are selected
only when both query IDs match; payout and query correlation domains stay separate.
Result/timeout ingress retains the secret, optional source policy, body limits,
strict duplicate-key parsing and redaction documented in [callback ingress](webhook-ingress.md).
Conflicting amounts, receipts, transaction IDs or result codes escalate for review.
An independent verified record may resolve an operation despite forged conflicting
hints; callback disagreement cannot override that record or change a terminal state.

`PayoutVerifier` is a trusted independent-evidence boundary, defaulting to
`UnavailablePayoutVerifier`. Implementations must obtain independently authenticated
provider records or reviewed statements and bind exact operation, payload hash,
amount and destination. Callback fields, status success and portal absence cannot
be copied into an attestation. Manual checks consume independently available
records; an implementation must not restart exhausted automatic status polling.
Paid requires an actual receipt. Failed requires documented definitive non-execution
or an audited authorized resolution. Operator authentication/evidence acquisition
remain stage 15; live independent settlement remains gated.

Resolution, final evidence, reservation release, audit and job completion commit
under the live generation lease in one transaction. Invalid evidence, a stale worker
or a failed final write releases nothing. Migration 0008 uniquely allocates a Paid
outbound receipt across payouts; conflicting legacy allocations fail migration
for operator review. Terminal states reject contradictory later transitions.

## Verification

`cargo test -p daraja-core --test collection reconcile_stage13 --locked --offline`
exercises real loopback payout/status/webhook HTTP and file-backed SQLite,
including abrupt child exits, byte equality, default/scoped/expired gates,
duplicate rejection, lost callbacks, unavailable IDs, late verified settlement,
concurrent workers, stale leases, query cancellation, contradictory callbacks,
deadlines, hard TTL and transactional fault recovery. Assertions check both CDK
Pending mapping and unreleased reservations on ambiguous paths. Local synthetic
evidence enables no [external capability](verification-gates.md).

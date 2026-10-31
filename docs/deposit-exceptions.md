# Deposit exceptions and refund reservations

Stage 10 extends `daraja-core::inbound` and `repository` with durable exception
handling. These are trusted library boundaries. Authenticated operator review, refund execution and FX re-quoting are described in
the [operator runbook](operator-runbook.md); daemon configuration parsing remains later work. Callbacks
and customer receipt-number submissions cannot create ownership attestations,
release holds or authorize refunds. Live independent verification and C2B
validation remain separately disabled capability gates.

## Verified balances and top-ups

Only `VerifiedDeposit` enters the confirmed ledger. Distinct verified receipts
allocated to one quote accumulate exactly once, using checked KES minor units.
Underpayment remains `UNDERPAID_HELD`; exact funding can settle only after all
verification, FX and hold predicates pass. Overpayment defaults to
`OVERPAID_HELD`, with no mint event. The specification's alternative policy of
settling the expected portion and creating refundable excess credit is optional
and **not implemented or enabled**.

Unknown references retain `ORPHAN_UNCLAIMED` custody, independent evidence and
the original shortcode/reference/rail/time correlation. A receipt number supplies
no ownership proof. Bound late and FX-expired receipts retain the more specific
`LATE_HELD` and `FX_EXPIRED_HELD` exception reasons. Already-paid quotes keep
additional distinct receipts in separate overpayment custody and never mint twice.

`Repository::set_deposit_policy` persists operator configuration and an audited
configuration snapshot. `DepositPolicy::default()` has zero KES grace and no
configured risk thresholds. A nonzero `kes_grace_seconds` admits KES receipts in
`[quote_expiry, quote_expiry + grace)`. Receipt time determines eligibility;
verification may occur later. This never extends sat/FX validity. Expired sat
receipts and stale FX remain held for explicit operator refund or fresh re-quote;
this stage does not silently reprice or reassign them.

When the separately verified C2B validation capability and held-top-up option are
enabled, validation admits only the current expected amount minus verified quote
allocations. The same KES grace applies. A validation response is a consistent
snapshot, not a reservation: simultaneous transfers can still overpay and are held.
Refund-reserved, paid, overpaid, fully funded and expired-FX quotes reject top-ups.

## Refund authorization and execution gate

`request_refund` is an authenticated-operator integration boundary. Its
`RefundAuthorization` must come from independently reviewed ownership and original
payer/destination evidence covering **every** requested receipt. The operator,
owner, evidence reference, destination and receipt set are persisted together.
Neither possession of a reference nor callback MSISDN authorizes a customer to
claim an orphan or choose where money is sent. No public customer claim endpoint
exists. Stage 15 must authenticate operators and obtain/review the actual evidence;
constructing this Rust value does not perform that review.

A refund reserves whole verified exception receipts. For quote-allocated funds,
the request must cover all quote allocations and the quote must be underpaid, overpaid or fully funded with expired FX. The quote atomically becomes `REFUND_HELD`: later receipts go into
separate exception custody, and neither settlement nor top-up admission can spend
the reserved balance. Existing receipt provenance/allocation is immutable.
Unique receipt reservations prevent another refund from using the same funds.
A competing top-up that fully funds the quote first makes the old refund request
ineligible. Transaction failure rolls back the request, reservations and quote.

The request freezes tariff version/fee, gross and exact net KES amounts. Fee at
or above gross is rejected without reserving funds. The net must be positive and
representable in provider whole KES; there is no rounding. Replaying the same
request ID and terms returns the original reservation; changed terms conflict.
Already-settled allocations cannot be refunded. Extra held receipts after mint
settlement can be reserved independently.

Reservations initially have `EXECUTION_GATED` status. Stage 15 binds a reviewed
reservation to one immutable B2C quote and executes the existing outbound saga;
`refund_status` projects the linked payout when present. Pending/manual outcomes
retain custody. Independent Paid resolution closes exception custody atomically;
receipt reservations remain permanent. See the operator runbook for failures,
replay and the conservative recovery limits.

## Payer risk holds and reversal incidents

The trusted verifier may attach `VerifiedPayer`: a stable pseudonymous payer
subject and independently established identity-verification status. Callback
MSISDNs do not establish either field. Older receipts decode with no payer.
Raw MSISDNs must never be used as these pseudonyms or written to application logs.

A nonzero `high_value_minor` requires positive `hold_seconds`,
`velocity_window_seconds` and `velocity_limit`. Each qualifying unverified deposit
extends the quote hold to at least its verification time plus the configured
hold. Counts use distinct qualifying receipts within the inclusive verification
window. Missing identities share a conservative `UNKNOWN` bucket. Above the
configured count, a durable velocity alert and manual settlement block are
recorded. Independently verified payer identities bypass this particular policy;
they do not bypass financial verification or quote holds. Operators must set
production thresholds deliberately; the software supplies no provider guarantee
or universal risk threshold.

Normal holds release from persisted verified funds after the deadline. Velocity
or reversal blocks require `release_deposit_hold` with authenticated operator and
review evidence. Release never bypasses exact value, independent verification,
FX expiry or the remaining timed hold. Repository settlement, callback workers
and explicit settlement all enforce these predicates.

`report_reversal` records an idempotent receipt/evidence incident and blocks any
unsettled quote. It neither reverts a mint event nor fabricates a successful
refund. Ecash already minted is bearer value and cannot be clawed back. Operators
must configure the collection shortcode for **Maker-Checker / Organization-Approved
Reversals Only** with the provider. This is an operator prerequisite outside
software control; simulated reversal tests cannot establish that configuration.

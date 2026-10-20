# Liquidity telemetry and admission

`daraja-core::liquidity` and the repository own separate collection and
disbursement float. This is a library boundary. Daemon configuration parsing,
worker supervision, HTTP metrics routing and authenticated operator delivery are
later stages. Collection funds never authorize a customer payout directly.

`Repository::configure_liquidity` requires explicit distinct shortcodes, an exact
KES minimum, maximum observation age and query interval. Account identities
cannot be rebound over existing accounting. A missing policy or an unavailable,
future-dated or stale observation closes admission. Policy changes cannot change
existing quote terms, reservations or payout identities.

## Balance observations and trust

Call `BalanceMonitor::run_once` from a periodic supervisor. Each account uses its
own configured AccountBalance client and OAuth role. A writer transaction commits
the unique local query, role/account, request time and exact credential-bearing
body before I/O. The transaction also consumes that account's polling interval.
Concurrent monitors cannot issue another request in that interval. A failed,
cancelled or abandoned request is never resent; the next interval starts a new
read-only query with a new identity. Persistence errors surface to the supervisor;
transport failures leave an unresolved query and existing observations age out.

Acknowledgment binds both provider IDs. Result and QueueTimeOutURL callbacks
stay in the admitted inbox, including replies arriving before acknowledgment.
`balance_targets` matches both IDs and only the two balance callback endpoints;
a wrong conversation or another payment rail cannot match. Recent unresolved
work is bounded to 100 queries per role and 100 observations per query per tick;
history remains durable. Older unprocessed evidence cannot prolong freshness.

`BalanceVerifier` is the trusted evidence boundary. It must establish the exact
account's **available KES**, at or after the recorded query start, using an
independently available provider source or reviewed statement. It must verify
account bucket, currency, available-balance field and provenance. A total, credit
limit, success code, correlation or callback secret alone is insufficient.
`BalanceEvidence` binds query, account, role, exact amount and evidence reference;
conflicting evidence fails closed. `UnavailableBalanceVerifier` is the default
implementation and never supplies a balance. No live provider balance-layout or
provenance guarantee is invented; the BALANCE-SOURCE gate remains disabled.
Tests supply an explicitly synthetic independent verifier and fixture parameter.
The opaque provider AccountBalance parameter remains available to a separately
verified adapter; an arbitrary column is never assumed spendable.

The effective snapshot is the verified query with the highest durable sequence,
not the last callback to arrive. Age starts at request preparation, not callback
arrival or verification. Delayed evidence cannot renew freshness, and an older
reply cannot replace a newer snapshot. Snapshot history and original query bodies
are persisted; query identity/body fields cannot be updated.

## Quote and payout accounting

`save_payment_quote` now atomically saves the immutable quote and reserves
principal plus frozen KES tariff. Under the same SQLite writer lock it enforces:

```text
verified disbursement balance >= minimum + new principal + tariff
                               + active quote holds + payout liabilities
```

There is no gap between admission and reservation. Replaying the identical quote
never extends its expiry; a changed quote conflicts. Undispatched holds cease to
count exactly at quote expiry. They remain in the ledger for audit. Quotes
predating migration 0009 have no newly granted hold; create a fresh admitted
quote rather than dispatching a historical unreserved quote.

Dispatch revalidates the quote and current float, then atomically converts its
hold into the existing payout reservation. The claim checks the wire source
against the disbursement policy for quoted payouts. The lower-level unquoted
repository path also checks float and cannot bypass capacity. After OAuth the
concrete outbound hook rechecks freshness, expiry, state and capacity immediately
before the single HTTP send. A guard/storage failure after the durable claim
retains Pending and its immutable identity; retries do not gain send authority.
Startup recovery and independent resolution follow the existing outbound saga.
The low-level transport remains separate from financial admission.

Dispatching, Pending, UnknownInFlight and manual payouts remain liabilities
regardless of elapsed time or quote expiry. Even if a provider snapshot may
already include a pending debit, the full reserve is subtracted conservatively.
Only independently definitive Failed releases capacity against an old snapshot.
A Paid payout continues to debit any snapshot whose query started at or before
its transactional reservation release. A verified query started strictly after
resolution can supersede that debit. This prevents old snapshots from reusing
spent funds. Full principal-plus-fee is retained without relying on actual-fee
refunds or provider read consistency stronger than the evidence contract.

## Optional linked sweeps

`OutboundPayments::sweep_if_needed(None)` keeps sweeps disabled and refreshes
operator alerts. Opt-in supplies `SweepConfig` and a separate source-account
payout client. The client must explicitly attest linked distinct shortcodes,
BusinessTransferFromMMFToUtility support and B2B caller-ID permission. The
configured source/utility must equal the persisted collection/disbursement pair;
the immutable wire body must name those accounts and the exact command/amount.
SWEEP, WIRE-MMF and WIRE-B2B-ID remain independently disabled until verified.
There is no substitution of another top-up command.

A sweep requires fresh observations for both accounts, low utility float and
source capacity including principal, fee, minimum and liabilities. Its operation
and request key derive from the source snapshot, and preparation uses the same
outbound claim, body, acknowledgment, callback, recovery and final-evidence paths
as payouts. No mint quote is fabricated for this internal transfer. Admission
allows one sweep per source snapshot, one unresolved sweep, a positive cooldown
and a checked rolling 24-hour gross budget. Attempts count even after definitive
failure. After a prior sweep resolves, both account queries must have started
strictly after resolution before another sweep can be admitted. A newly refreshed
source alone cannot repeatedly top up an old utility observation.

Sweep HTTP errors, timeouts, crashes, queue timeout and manual escalation retain
Pending and source liability. No automatic probe is authorized for the sweep
command. Concrete transaction-ID status reconciliation and independently verified
late resolution remain available through the shared worker. Success never credits
the utility ledger directly; a new independently verified balance is required.
Unlinked, disabled, unknown or stale accounts receive low-float/unavailable alerts
and instructions to verify the disbursement account and initiate an operator
bank/PesaLink transfer. Delivery and operator authentication remain stage 15.

## Metrics and alerts

`liquidity_metrics(now)` returns Prometheus text. Missing balance gauges are
omitted; unknown age is `+Inf`. Stale known gauges remain visible alongside age
and active alerts; consumers must never use a balance gauge alone for admission.

| Metric | Meaning |
|---|---|
| `daraja_b2c_utility_balance_kes` | Latest verified disbursement observation |
| `daraja_c2b_collection_balance_kes` | Latest verified collection observation |
| `daraja_balance_age_seconds{account}` | Conservative query age |
| `daraja_pending_liability_kes{account}` | Pending plus Paid debits not superseded by this snapshot |
| `daraja_quote_reservations_kes{account}` | Active undispatched quote holds; zero for collection |
| `daraja_liquidity_alert{account}` | Unknown/stale or available float below minimum |

The only labels are `account="collection"` and `account="disbursement"`.
No phone, secret, shortcode, payment or query ID appears in metric labels.
`refresh_liquidity_alerts` durably upserts current bounded alerts;
`liquidity_alerts` exposes reason, active/recovered state and update time.
`FloatStatus::operator_instruction` supplies the bank/PesaLink guidance.

Local acceptance uses file-backed SQLite and loopback HTTP:

```sh
cargo test -p daraja-core --test liquidity --locked --offline
cargo test -p daraja-core --test collection liquidity --locked --offline
```

These establish no live balances, linked-account permissions, duplicate guarantee
or money movement. All external gates remain disabled.

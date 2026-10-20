# Tariffs and frozen valuation

Owners: `daraja-core::{tariffs,fx,valuation}`, inbound checkout and repository.
These are library contracts. Provider dispatch, daemon configuration and CDK RPC
services remain later stages. All external verification gates remain disabled.

`Tariffs::new` requires separate operator-supplied B2C and B2B schedules. Phone
payments select B2C; Till and PayBill select B2B. No production prices are shipped.
Test schedules and rates are explicitly synthetic. Collection fees are separately
configured in `CheckoutConfig.tariff`; they are not a B2C disbursement fee.

Bands use whole-KES principal intervals **[min, max]**, both ends inclusive.
Adjacent bands must start at the previous maximum plus one. The first minimum
and last maximum define supported amounts; amounts outside them, fractional-KES
principals and zero payouts fail. Schedule dates are UTC seconds **[from, until)**,
end exclusive. Adjacent versions must meet exactly; overlaps, gaps, empty or
reversed intervals, duplicate version IDs within a rail and wrong fee units fail.
Fees are nonnegative exact KES minor amounts. `Amount::kes_decimal` accepts fixed
unsigned decimals with at most two places, rejecting negative values, exponents,
excess precision and overflow. Zero fees are allowed when explicitly configured.

`PaymentQuote` freezes normalized destination, whole-KES principal, tariff
version/fee/effective dates, rational FX source/rate/expiry, wallet principal,
total reserve and expiry. Expiry is the earliest requested, tariff-version or FX
expiry. `save_payment_quote` durably saves it; replay requires identical terms,
even after expiry. A conflicting reload cannot overwrite it. SQLite forbids
quote updates/deletes. `DispatchRecord::with_quote` binds the saved quote to the
canonical payload, tariff and exact principal-plus-fee KES reservation.
`Payout::dispatching` and repository admission revalidate it and reject expired
new dispatches. Expiry never changes an already dispatched payout or authorizes
retry, failure or proof release. Stage 12 must use this quoted path and recheck
the clock immediately before I/O; legacy unquoted domain records remain readable
for compatibility, not an outgoing service implementation.

## Rounding and fee accounting

The FX rational is `n/d` sat per KES minor unit. Products use checked-width
`u128`; results must fit `u64`. Zero rates, zero divisors, wrong units and
out-of-range results fail.

| Operation | Rounding |
|---|---|
| Issue sat from net KES minor `k` | floor(`k*n/d`) |
| Back requested sat `s` | ceil(`s*d/n`) minor KES, add collection fee, round gross up to whole KES |
| Value sat for a payout budget | floor(`s*d/n`) minor KES |
| Pay principal from that budget | Largest supported whole-KES principal whose fee also fits; inspect every band |
| Reserve/debit KES principal plus fee in sat | ceil(`(principal+fee)*n/d`) |
| Final actual cost in sat | Same ceiling using the frozen rate and independently established actual fee |
| Local CDK `kes` charge | Round total minor KES up to whole KES |

Incoming requested sat credit can be below floor(net backing converted to sat):
whole-KES collection rounding leaves unissued dust. KES credit remains exactly
gross minus the configured collection fee. No overpayment is silently issued.
The existing receipt allocation, risk, grace and exception rules still apply.
Expired incoming FX remains held for reviewed refund/re-quote, including on-time
receipts verified after FX expiry. A hold cannot refresh a rate.

The pinned CDK protobuf has `fee` and `total_spent` amount messages. Its
[paid melt finalization](https://github.com/cashubtc/cdk/blob/9f6d3a3734d7223d10b5553e3591d3f9d5c6c904/crates/cdk/src/mint/melt/melt_saga/mod.rs)
uses the payment's total spent when issuing change and validates the quote unit.
Our local wire mapping defines `kes` as whole KES and `sat` as sats; internal
`KesMinor` must never be copied directly into a CDK `kes` amount.
`cdk_principal`, `cdk_fee_reserve` and `cdk_total_spent` provide that conversion.
Reserve is rounded total minus rounded principal, so returning change cannot
undercharge the mint. CDK service integration and proof-state acceptance remain
stages 17–19; these helpers establish no deployed mint support.

`record_payout_fee` is a trusted independent-evidence boundary available only
for a verified PAID quoted payout. It records actual fee and conservative total
spent immutably. Missing actual-fee evidence consumes the entire frozen reserve.
An observed fee above the quoted reserve is retained with NULL total_spent and
returns SettlementHeld for reconciliation; it never authorizes compensation or
an invented successful cost. Identical audit replay succeeds; conflicting fee
evidence requires operator reconciliation. Do not expose this method to callbacks
or unauthenticated clients. Refund execution remains gated; these methods do not
submit refunds or payouts.

## Oracle and capability admission

`FxOracle` is asynchronous and pluggable. `OfflineOracle` supplies explicit local
fixtures or unavailable behavior. `CoinbaseOracle` implements the official
[Coinbase v2 exchange-rates contract](https://docs.cdp.coinbase.com/coinbase-app/track-apis/exchange-rates),
reviewed 2026-10-20: GET `/v2/exchange-rates?currency=BTC`, require
`data.currency=BTC` and a positive fixed-decimal string `data.rates.KES`.
A missing KES rate is unavailable; currency coverage is never assumed. Endpoint,
TTL, maximum response age and request timeout are configurable. A production
endpoint must use HTTPS; loopback HTTP exists for boundary fixtures. There are
no credentials, authentication requirement, redirects, retries or proxy inheritance.
The response body is limited to 64 KiB, the whole request to at most 20 seconds.

For price `p/q` KES per BTC the snapshot is `1_000_000*q/p` sat per KES minor.
Decimal parsing preserves the string exactly, accepts at most eight places and
rejects signs, exponents, nulls, duplicate KES fields and unsupported shapes.
Oracle errors never select a default rate or reuse expired data.

The official response has **no market-update timestamp**. Freshness is explicitly
HTTP observation/cache age, not proof of exchange execution price or market tick
freshness. The HTTP Age header must parse when present and remain below the
configured age bound. Remaining quote life is min(TTL, max_age - Age), starting
at request initiation; slow responses cannot extend it. Require `0 < TTL <=
max_age <= 300s`. If market-update-time provenance or executable hedging is a
release requirement, a provider supplying that evidence is needed before sat is
enabled. No live FX request or money movement was used for local acceptance.

Checkout defaults to no externally advertised CDK units.
`with_verified_kes_capability` and `with_verified_sat_capability` are trusted
configuration attestations for independently established deployed mint support;
they do not discover support or open the capability register. `enabled_units`
advertises only explicitly verified units and additionally requires a currently
valid oracle for sat. Local KES checkout remains usable by synthetic library
callers. Sat creation uses `create_with_oracle`; no oracle means unavailable.
The clock is checked again after oracle I/O. All returned quotes retain their
terms during oracle loss, expiry, reload or restart; replay performs no new I/O.
New bindings hash caller inputs separately from pricing configuration. Stage 08
bindings are recognized by reconstructing their frozen configuration and original
preparation time, preserving old hashes and refusing changed caller inputs.

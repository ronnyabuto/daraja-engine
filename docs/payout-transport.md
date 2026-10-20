# Payout and balance transport contracts (stage 06)

`daraja-core::payout` owns typed payout destinations, B2C/B2B command selection,
AccountBalance and untrusted result/timeout observations. It shares bounded HTTP,
authorization and strict response decoding with collection through `transport`.
It has no retry loop, settlement authority or payment-state projection.

## Wire contracts and gates

| Command | Path | Wire identifiers / fields | Default |
|---|---|---|---|
| BusinessPayment | `/mpesa/b2c/v3/paymentrequest` | `InitiatorName`, `OriginatorConversationID`, `Occassion`; shortcode PartyA, normalized phone PartyB | 2547 supported; 2541 gated |
| BusinessBuyGoods | `/mpesa/b2b/v1/paymentrequest` | `Initiator`, `SenderIdentifierType=4`, **`RecieverIdentifierType=2`**, till PartyB, corporate `AccountReference=Payout` | Command and caller-ID gates off |
| BusinessPayBill | same B2B path | sender 4 / receiver 4, paybill PartyB, destination AccountReference | Command and caller-ID gates off |
| BusinessTransferFromMMFToUtility | same B2B path | sender 4 / receiver 4; configured linked source/utility, explicit account reference | Sweep, exact-command and caller-ID gates off; no linked accounts |
| AccountBalance | `/mpesa/accountbalance/v1/query` | `Initiator`, `IdentifierType=4` for shortcode or gated 2 for till; no caller payout ID | Till gate off |

Every request includes `SecurityCredential`, `PartyA`, `Remarks`, `ResultURL` and
**`QueueTimeOutURL`**. Payouts include a positive exact whole-KES `Amount` and
`CommandID`. Balance has no amount. B2B has no `Requester`, customer name or phone
field; remarks and remitter information come from corporate configuration. The
PayBill reference is user-provided destination data and can itself identify an
account; callers must not append unrelated customer identifiers.

Stage 01 documented the B2C v3 fields. Stage 06 revisited the current official
[BuyGoods](https://developer.safaricom.co.ke/apis/BusinessBuyGoods),
[PayBill](https://developer.safaricom.co.ke/apis/BusinessPayBill),
[B2C](https://developer.safaricom.co.ke/apis/BusinessToCustomer) and
[balance](https://developer.safaricom.co.ke/apis/AccountBalance) pages on 2026-10-20;
they returned documentation shells. The pinned historical
[primary PHP SDK](https://github.com/safaricom/mpesa-php-sdk/blob/5effb808e79ba6b3d8631ef83c2b9b31fa450152/src/Mpesa.php)
confirms the B2B/base balance field spellings and paths, not current product
acceptance. Identifier/command combinations above remain gated candidates where
current evidence is absent. The accessible
[top-up page](https://developer.safaricom.co.ke/apis/B2CAccountTopUp) specifies
`BusinessPayToBulk`; it does not establish support for the requested MMF command.
There is no fallback to that different financial operation.

Capabilities are operator attestations for this client's endpoint/environment;
construct a new client for a different environment. See the independent
[verification gates](verification-gates.md). No gate was enabled by these tests.
Command support/caller-ID acceptance never enables duplicate probing. Sweep also
requires explicitly linked **distinct** accounts whose source equals PartyA and
utility equals the requested destination. Configuration parsing, authorization,
liquidity admission and optional bounded sweep orchestration are supplied by
[liquidity](liquidity.md); daemon scheduling remains a later stage.

## Validation and credentials

`Destination::parse` accepts exact ASCII `07xxxxxxxx`, `01xxxxxxxx`,
`2547xxxxxxxx`, `2541xxxxxxxx`, `TILL:<number>` and
`PAYBILL:<shortcode>:<account_ref>`. Domestic phones normalize to 254; sending a
2541 phone additionally requires `phone_01`. Bare merchant numbers, plus prefixes,
spaces, control characters, Unicode digits and extra delimiters fail closed.
The supported local policy subset is nonzero-leading 5–7 digit tills, 5–6 digit
shortcodes, and 1–13 ASCII alphanumeric/underscore/hyphen reference characters.
Caller IDs are nonblank and at most 64 characters. These are local safety ceilings,
not independently verified universal provider limits. Product/account approval
must confirm this subset (or narrow it in caller admission) before gate enablement;
other legitimate provider formats remain unsupported until separately verified.
`max_amount_kes` is mandatory, positive and must be no greater than the approved
product/account limit. Fractional KES, zero, excess, wrong units and overflow fail
before persistence/OAuth/HTTP. Account identifiers supplied directly in enums are
revalidated at dispatch. Balance account must equal the configured remitter account.

`Remitter::new` encrypts the initiator password once with the stage 04
`SecurityCredentialKey`, rejecting a certificate/environment mismatch. The encrypted
value is retained for immutable request preparation. Payouts require
`DisbursementApp`; balance explicitly selects collection or disbursement credentials.
OAuth and provider roots must share an origin. Roots must match the selected
Safaricom environment; numeric loopback fixtures are allowed. Operator certificate
provenance and exact shortcode/app/role permissions remain external requirements.
Secrets, callback URLs, destination phones and account references use redacted
Debug. The transport never logs request bodies or nested provider error text.

## Persistence, outcomes and observations

There is no no-op production persistence implementation. `PayoutPersistence` must:

1. Commit caller ID, command, exact `PreparedPayout::wire_body`, callback binding and
   unresolved recovery work before returning from `prepare`. Reject conflicting
   or already-dispatched identity; do not authorize replay merely because bytes
   match. The [stage 12 saga](outbound-dispatch.md) owns reservation, operation identity and idempotency.
2. Atomically bind the acknowledgment's caller, provider originator and conversation
   IDs without replacing an early callback or conflicting existing ID. Proxy IDs
   may differ from the caller ID. Optional synchronous TransactionID, request ID,
   code and HTTP status survive in `Outcome`.

Cancellation or failure after preparation leaves an unresolved record. A failed
acknowledgment commit retains provider evidence in the returned error. No financial
state changes happen here. A request/body timeout, malformed acknowledgment, 500/503,
transport failure or lost hook result never proves failure of a payout.

`PayoutError` carries caller ID, original transport evidence and a disposition:
`NotDispatched` refers only to this invocation's local validation/preparation
failure, never proof that a prior operation was not sent; `Duplicate` means the
literal `500.002.1001` was observed, without attesting earlier execution/finality;
`Uncertain` is the default after preparation. `ProvenRejection` requires an explicit
command-specific code allowlist supported by independent endpoint/environment
non-execution evidence. That list defaults empty; success, duplicate and throttling
codes cannot be added. HTTP 5xx always remains unresolved even with an allowlisted
rejection code. None of these observations alone supplies a CDK terminal state.

`BalancePersistence::prepare` must commit the unique local query ID, immutable
account/body, timestamp, secret callback binding and scheduled asynchronous work
before OAuth/I/O. `acknowledge` durably binds both provider IDs, preserving early
observations. Failures leave the query unresolved. This matches the existing
TransactionStatus prepare/ack contract; stage 06 adds no worker, migration or
periodic schedule. Concrete durable worker/restart wiring belongs to stages 09/13/14.

`PayoutResult` preserves ResultCode, optional ResultType, both correlation IDs,
TransactionID and redacted result parameters. It reconciles TransactionReceipt,
ReceiptNo and TransactionID without inventing missing values; contradictory receipts
or Amount/TransactionAmount fail closed. `BalanceResult` retains opaque redacted
AccountBalance parameters for the later liquidity parser. Result/timeout DTOs are
untrusted observations; even zero ResultCode needs finality, provenance and binding
verification. Timeout DTOs never mean financial failure. Duplicate JSON/parameter
keys, floats, invalid scalar fields and oversized bodies fail closed.

## Local verification

`cargo test -p daraja-core --test collection --test redaction --locked` runs the
shared loopback HTTP harness, stage 06 contracts in `tests/support/payout.rs` and
captured TRACE logging. Coverage includes every command, all phone forms, B2B
identifier pairs, both balance apps/identifier types, exact prepared/wire body
equality, RSA fixture decryption, distinct caller/provider IDs, destination/amount
rejection, capability combinations, persistence failures, malformed acknowledgments,
duplicate code, real request/body deadlines, 401 and 500/503 with one dispatch only.
B2B serialization contains no customer phone field; TRACE omits customer phone,
private references, initiator, credentials and full callback URLs. Fixtures are
synthetic and prove no live provider behavior or combined mint integration.

Stage 12 supplies the concrete repository-backed hooks in
`outbound::OutboundPayments`. `PreparedPayout::endpoint` freezes the actual target
with the body before I/O. Transport classifications remain observations; the saga
never derives terminal failure solely from an HTTP response or error code.

Stage 13 adds the internal frozen-byte probe boundary and concrete status-query
persistence under [outbound reconciliation](outbound-reconcile.md). Only the leased,
evidence-approved worker authorizes a probe; ordinary MakePayment replay still
never authorizes another send.

Stage 14 supplies concrete balance query persistence and conservative snapshot
processing under [liquidity](liquidity.md). `PayoutPersistence::before_send` allows
the durable outbound hooks to recheck float after OAuth; transport-only hooks
retain their existing behavior. New payout admission always includes liquidity
checks in the repository, and a failed guard never releases a possibly sent payout.

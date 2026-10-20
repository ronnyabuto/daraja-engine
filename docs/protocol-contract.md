# Protocol contract

Recorded 2026-10-20 for stage 00. This document pins build and wire inputs;
it does not establish provider behavior or CDK financial compatibility.

## Build inputs

| Input | Exact selection |
|---|---|
| Rust / Cargo | 1.93.0, Edition 2024, resolver 3 |
| Tokio | 1.48.0 |
| axum | 0.8.9 |
| tonic / tonic-prost / tonic-prost-build | 0.14.2 |
| prost / prost-build / prost-types | 0.14.1 |
| SQLx | 0.8.6, defaults off, runtime-tokio + sqlite |
| protoc-bin-vendored | 3.2.0; libprotoc 31.1 on x86_64 Linux |
| Transitive code generation | tonic-build 0.14.6, prost-derive 0.14.4 (Cargo.lock) |
| SHA-256 implementation | sha2 0.10.9 |

Direct dependencies use exact constraints; Cargo.lock records transitive versions
and registry checksums. Rust 1.93.0 is a deliberate supported build baseline,
not a claim that it is the latest release. No external `protoc`, database server,
Daraja credentials or CDK checkout is needed to build.

Compatibility sources inspected (2026-10-20):

- [Rust 1.93.0 release](https://blog.rust-lang.org/2026/01/22/Rust-1.93.0/).

- [Tokio 1.48.0](https://docs.rs/tokio/1.48.0/tokio/): runtime and macros features.
- [axum 0.8.9](https://docs.rs/axum/0.8.9/axum/): Tokio/Hyper integration.
- [SQLx 0.8.6](https://docs.rs/sqlx/0.8.6/sqlx/): explicit runtime-tokio;
  SQLite is bundled by the selected sqlite feature. PostgreSQL is stage 16.
- [tonic-prost-build 0.14.2](https://docs.rs/tonic-prost-build/0.14.2/tonic_prost_build/)
  and [prost-build 0.14.1 Config](https://docs.rs/prost-build/0.14.1/prost_build/struct.Config.html):
  generate clients/servers using `compile_with_config`, with an explicit
  `protoc_executable` rather than changing the process environment in Edition 2024.
- [protoc-bin-vendored 3.2.0](https://docs.rs/protoc-bin-vendored/3.2.0/protoc_bin_vendored/):
  supplies host compiler binaries; support is limited to its supported host platforms.
- Pinned CDK root Cargo.toml uses tonic/tonic-prost/tonic-prost-build 0.14.2,
  prost 0.14, axum 0.8 and Rust MSRV 1.85.0. This workspace compiles only its
  wire types, not the full CDK mint; stage 01/19 must verify the latter separately.

## CDK provenance

- Upstream: <https://github.com/cashubtc/cdk>
- Release tag: **v0.18.2** (observed latest stable release on 2026-10-20).
- Exact resolved commit: **9f6d3a3734d7223d10b5553e3591d3f9d5c6c904**.
- Original path: `crates/cdk-payment-processor/src/proto/payment_processor.proto`.
- [Immutable source](https://github.com/cashubtc/cdk/blob/9f6d3a3734d7223d10b5553e3591d3f9d5c6c904/crates/cdk-payment-processor/src/proto/payment_processor.proto).
- Local copy: `crates/cdk-daraja-processor/proto/payment_processor.proto`.
- SHA-256: `1b618a8f0bb0ecfc704e3b856697b524843bb944ae6bcde848698c06cd7b37ee`.
- Copy is byte-for-byte unchanged and has no proto imports.
- License: upstream `MIT OR Apache-2.0`, per root LICENSE.md. Original license
  notice and both licenses are retained in `proto/upstream/`; upstream MIT
  copyright notices are preserved there. Workspace licenses cover original work.

`build.rs` verifies this checksum and generates both Rust client/server types and
a file descriptor set into Cargo OUT_DIR. Generation needs no GitHub access.
The application does not yet implement the generated service trait.

## PRD mismatch and proposed compatibility decision

PRD sections 4, 5.1 and 7 name `WaitIncomingPayment`; the authoritative PDF
(pages 5–8 extracted for verification) agrees with the supplied text on that name.
The actual service package is `cdk_payment_processor`, service
`CdkPaymentProcessor`, with these seven RPCs:

| RPC | Request | Response |
|---|---|---|
| GetSettings | EmptyRequest | SettingsResponse |
| CreatePayment | CreatePaymentRequest | CreatePaymentResponse |
| GetPaymentQuote | PaymentQuoteRequest | PaymentQuoteResponse |
| MakePayment | MakePaymentRequest | MakePaymentResponse |
| CheckIncomingPayment | CheckIncomingPaymentRequest | CheckIncomingPaymentResponse |
| CheckOutgoingPayment | CheckOutgoingPaymentRequest | MakePaymentResponse |
| WaitPaymentEvent | EmptyRequest | stream PaymentEventResponse |

There is **no WaitIncomingPayment RPC** in this pin. The similarly named
`WaitIncomingPaymentResponse` message is an incoming event payload;
`PaymentEventResponse.event` also includes `payment_successful` and `payment_failed`.
Custom options include mint-generated `quote_id` correlation, amount and
`extra_json`; these must not be replaced with invented PRD DTOs.

Accepted decision in stage 01: target the exact v0.18.2 service and route
incoming notifications through `WaitPaymentEvent.payment_received`; retain outgoing
event semantics for explicit review. Do not rename the RPC or fabricate a legacy
compatibility service. Only type generation is accepted here; event mapping,
proof locks, custom `mpesa`/KES/sat handling and all adapter behavior remain gated
on the evidence below and stages 17–19 implementation.

## Capability boundary

No stage 00 code opens a socket, reads credentials, calls a provider, dispatches
money or settles a quote. Configuration is a documentation template only and all
listed unverified capabilities are false. Stage 01 creates the detailed live gate
register; no duplicate-rejection, regulatory, receipt verification or proof-lock
guarantee has been established by this build.

## Stage 01 research (2026-10-20)

The PDF sections 2, 4, 5.2, 5.3 and Phase 0 were compared with the text using
`pdftotext -layout`. The following decisions override unsupported assertions in
the PRD, while preserving its financial invariants. Documentation evidence,
executed local evidence and external verification are distinct. Portal URLs are
mutable; observations are dated, not represented as immutable provider releases.
Historical primary SDK pins: Java `0a3667515b2d8b4abff7a454e8d2934cdb3c1d18`,
PHP `5effb808e79ba6b3d8631ef83c2b9b31fa450152` (resolved with `git ls-remote`).
Direct portal fetches often returned a documentation shell; indexed primary-page
text supplied the observations cited below. No authenticated portal access occurred.

### Provider request contracts

Only `https://sandbox.safaricom.co.ke` is used by the experimental harness.
Production endpoints are outside this stage's authorization. Requests use OAuth
Bearer credentials; this authenticates requests to the provider, not callbacks.

| Operation | Path and relevant fields | Evidence / limitation |
|---|---|---|
| STK creation | `/mpesa/stkpush/v1/processrequest`; `BusinessShortCode`, `Password`, `Timestamp`, `TransactionType`, `Amount`, `PartyA`, `PartyB`, `PhoneNumber`, `CallBackURL`, `AccountReference`, `TransactionDesc` | [Current STK primary page](https://developer.safaricom.co.ke/apis/MpesaExpressSimulate); [Safaricom Java SDK](https://github.com/safaricom/mpesa-java-sdk/blob/0a3667515b2d8b4abff7a454e8d2934cdb3c1d18/src/main/java/Mpesa.java). SDK is historical, not a current v3 payout reference. |
| STK query | `/mpesa/stkpushquery/v1/query`; `BusinessShortCode`, `Password`, `Timestamp`, `CheckoutRequestID` | Same [primary SDK](https://github.com/safaricom/mpesa-java-sdk/blob/0a3667515b2d8b4abff7a454e8d2934cdb3c1d18/src/main/java/Mpesa.java), method `STKPushTransactionStatus`; query is checkout-specific, not manual C2B lookup. |
| C2B registration | `/mpesa/c2b/v1/registerurl`; `ShortCode`, `ResponseType`, `ConfirmationURL`, `ValidationURL` | [Current C2B page](https://developer.safaricom.co.ke/apis/CustomerToBusiness) and [historical primary PHP SDK](https://github.com/safaricom/mpesa-php-sdk/blob/5effb808e79ba6b3d8631ef83c2b9b31fa450152/src/Mpesa.php). Version entitlement and observed callbacks still require account verification. |
| B2C | `/mpesa/b2c/v3/paymentrequest`; `OriginatorConversationID`, `InitiatorName`, `SecurityCredential`, `CommandID=BusinessPayment`, `Amount`, `PartyA`, `PartyB`, `Remarks`, `QueueTimeOutURL`, `ResultURL`; sample uses `Occassion` | [Current B2C page](https://developer.safaricom.co.ke/apis/BusinessToCustomer). Sample documents `500.002.1001` duplicate rejection. This does not prove retention duration, payload-conflict semantics, scope or exactly-once behavior. |
| B2B | `/mpesa/b2b/v1/paymentrequest`; `Initiator`, `SecurityCredential`, `SenderIdentifierType`, **`RecieverIdentifierType`**, `CommandID`, `Amount`, `PartyA`, `PartyB`, `AccountReference`, `Remarks`, callback URLs | [Current primary B2B top-up page](https://developer.safaricom.co.ke/apis/B2CAccountTopUp) confirms the endpoint and misspelled receiver field for its own command. [Buy Goods](https://developer.safaricom.co.ke/apis/BusinessBuyGoods) / [Pay Bill](https://developer.safaricom.co.ke/apis/BusinessPayBill) direct reads exposed only a shell. Their exact current contracts remain an access gate. |
| TransactionStatus | `/mpesa/transactionstatus/v1/query`; `Initiator`, `SecurityCredential`, `CommandID=TransactionStatusQuery`, `TransactionID` or **`OriginalConversationID`**, `PartyA`, `IdentifierType`, `ResultURL`, `QueueTimeOutURL`, `Remarks`, `Occasion` | [Current status page](https://developer.safaricom.co.ke/apis/TransactionStatus) explicitly describes receipt or conversation lookup. Its parameter table inconsistently spells `QueueTimeoutURL`; sample uses `QueueTimeOutURL`. Test the sample spelling, do not normalize wire names globally. |

B2B experiment candidates preserve PRD `BusinessBuyGoods` with sender 4 / receiver
2, and `BusinessPayBill` with sender 4 / receiver 4 plus account reference. They
are **candidates**, not independently verified current command contracts. In
particular, B2B acceptance of a client `OriginatorConversationID` and duplicate
rejection are unverified. The harness requires one for identity-equality testing;
it does not imply the endpoint honors it. Do not silently fall back to a fresh ID.

The B2C page's duplicate sample is stronger evidence than the PRD's assertion
that sandbox verification already occurred, but is still documentation only.
No duplicate rejection is evidence of completion. Keep caller identity, proxy
identity and provider conversation identity separately; their equality is not
established. Conversation-only status lookup is documented with the different
request spelling above; which original ID is accepted needs endpoint/environment
evidence. Until then, receipt lookup is the documented conservative candidate,
and an attempt with no receipt may require portal reconciliation.

### Callback fields and evidence limits

| Callback/query | Fields to inspect | What cannot be inferred |
|---|---|---|
| STK callback | `Body.stkCallback`: `MerchantRequestID`, `CheckoutRequestID`, `ResultCode`, `ResultDesc`; success `CallbackMetadata.Item` entries named `Amount`, `MpesaReceiptNumber`, `TransactionDate`, `PhoneNumber` | [Current STK page](https://developer.safaricom.co.ke/apis/MpesaExpressSimulate). Failure need not carry metadata. No demonstrated callback `BusinessShortCode`; bind shortcode via the stored checkout/quote. |
| C2B observation | `TransID`, `TransAmount`, `BusinessShortCode`, `BillRefNumber`, `TransTime`, `TransactionType`, `MSISDN` | [Historical primary callback parser](https://github.com/safaricom/mpesa-php-sdk/blob/5effb808e79ba6b3d8631ef83c2b9b31fa450152/src/TransactionCallbacks.php). This is not the STK shape: do not require `ResultCode` or `MpesaReceiptNumber`. Current privacy/version forms need actual fixtures. |
| Payout result | `Result` envelope with `ResultCode`, `ResultDesc`, `OriginatorConversationID`, `ConversationID`, `TransactionID`, `ResultParameters.ResultParameter` | [Primary callback parser](https://github.com/safaricom/mpesa-php-sdk/blob/5effb808e79ba6b3d8631ef83c2b9b31fa450152/src/TransactionCallbacks.php); [B2C page](https://developer.safaricom.co.ke/apis/BusinessToCustomer). Query/callback identities are not interchangeable. |
| Status result | `Result` with correlation, result code, `TransactionID`, keyed result parameters such as `ReceiptNo` | [Status page](https://developer.safaricom.co.ke/apis/TransactionStatus). Sample duplicates `DebitPartyName`, and one object has repeated `Key` members. No dependable amount field or receipt-to-amount binding is established. Reject duplicate JSON keys and contradictory fields. ResultCode success alone is insufficient. |
| STK query result | A checkout status observation over provider HTTPS | No current complete response schema was obtainable from the query page. No evidence it returns/verifies the callback's receipt and amount. Keep receipt/amount attestation gated. |

The C2B page confirms external validation is disabled by default and needs
provider activation. Registration alone does not activate it. Use `Cancelled`
as the candidate failure default; verify timeout behavior on the actual shortcode.
Its documented response is string `ResultCode` with `ResultDesc`; `0` accepts,
`C2B00011` rejects MSISDN, `C2B00012` account, `C2B00013` amount, `C2B00014` KYC,
`C2B00015` shortcode, `C2B00016` other error. Its v1/v2 FAQ describes hashed
versus masked MSISDN. None supplies a signature. Source:
[Safaricom C2B](https://developer.safaricom.co.ke/apis/CustomerToBusiness).

### Settlement trust decision

Threat model: an internet caller may forge/replay callbacks, discover a callback
URL in logs, or race a legitimate receipt with an invented quote/amount. A query
correlation ID is visible application data, not an authenticator. A fresh random
per-query callback secret, constant-time secret checking, TLS termination,
provider-approved network restrictions/VPN and strict binding to a durable query
reduce exposure; HTTPS with ordinary server authentication does not authenticate
the webhook sender. No retrieved primary contract established signed callbacks,
an authenticated client certificate, or an authoritative published IP range.

Consequently asynchronous TransactionStatus is not yet independently trustworthy
settlement evidence. An attacker with query callback access can forge it too.
Do not label the same callback channel "out of band" solely because an OAuth
request preceded it. Require an operator-approved provenance mechanism plus
independently attested receipt, amount, destination/shortcode, finality and quote
binding. Provider portal/statement evidence with authenticated operator review is
a candidate fallback, not implemented automation. If a status source omits amount
or receipt, a success cannot attest those fields from a separate callback.

No callback alone settles a mint. Preserve unverified observations separately
from transactional receipt allocation; a spoof must not reserve a legitimate
receipt forever. A timer, query timeout, worker crash, missing result or manual
escalation never releases a possibly dispatched payout. Both `Failed` **and
`Unpaid`** from a CDK backend can cause compensation; neither may mean "unknown".
The machine register keeps independent settlement evidence disabled.

### NUT and custom method/unit compatibility

Pinned NUT revision: **8bde3c0c3684430d852ab543ac8ca72913770dc0** (resolved
2026-10-20). [NUT-04](https://github.com/cashubtc/nuts/blob/8bde3c0c3684430d852ab543ac8ca72913770dc0/04.md)
describes general method-specific mint routes/settings.
[NUT-05](https://github.com/cashubtc/nuts/blob/8bde3c0c3684430d852ab543ac8ca72913770dc0/05.md)
defines method routes and `[a-z0-9_-]+` method names, matching input units and
pending melt behavior. Neither defines an M-Pesa wire method. `mpesa` is a local
custom extension; NUT conformance does not establish support in existing wallets.

At the pinned CDK revision, `crates/cashu/src/nuts/nut00/mod.rs` contains
`PaymentMethod::Custom(String)`, method validation and `CurrencyUnit::Custom`;
custom units normalize to lowercase. Thus use `mpesa`, `kes`, `sat` on the wire;
KES is a custom unit while sat is built in. `hashed_derivation_index` handles
custom units; the deprecated `derivation_index` returns None for them. Check
keyset/signatory configuration, routes and supported NUT-04/05 settings in the
actual mint in stage 19; serialization is not evidence of successful issuance.
`SettingsResponse.custom` and custom incoming/outgoing protobuf options preserve
extra JSON, amount and mint quote correlation. The incoming custom options have
no method-name field in this pin: backend registration/endpoint configuration
must disambiguate method and unit; never invent a field number.

### Proof-lock source trace and executable boundary

All paths below are relative to [pinned CDK](https://github.com/cashubtc/cdk/tree/9f6d3a3734d7223d10b5553e3591d3f9d5c6c904),
not the unpinned default branch:

| Boundary | Source / observation |
|---|---|
| gRPC protocol | `crates/cdk-common/src/lib.rs`: version `4.0.0`; `grpc.rs`: header `x-cdk-protocol-version`. Actual upstream client uses version interceptor and supports mTLS. The local generated-client fixture does not exercise that interceptor. |
| Transport errors | `crates/cdk-payment-processor/src/proto/client.rs:make_payment`: tonic errors map to payment errors (`Custom`, or message-based already-paid/pending variants); `check_outgoing_payment` errors are not terminal status responses. Stream disconnect marks stream inactive. |
| Before dispatch | `crates/cdk/src/mint/melt/melt_saga/mod.rs:setup_melt`: proofs/quote Pending and durable saga. `make_payment` writes the payment-attempt marker before backend I/O. |
| Pending / gRPC error | Same file: `verify_ambiguous_payment`, `handle_payment_error`, `check_payment_state`: recheck backend state after non-paid response/error. Pending is preserved; Unknown cannot erase an acknowledged Pending. Definitive Unpaid/Failed can release proofs even after initial Pending. Missing lookup IDs cannot justify failure. |
| Recovery | `crates/cdk/src/mint/start_up_check.rs:recover_from_incomplete_melt_sagas`, `check_melt_payment_status`; `saga_recovery.rs:process_melt_saga_outcome`: Pending/Unknown keep locks, terminal Paid finalizes, authoritative Unpaid/Failed compensates. Slow checks defer startup. |
| Real upstream tests | `crates/cdk/src/mint/melt/tests/pending_async_melt_tests.rs`: pending/unknown/error, successful-event recovery, bounded waiting, durable lookup IDs, startup with stalled backend; `mint/saga_recovery.rs` regression tests cover compensation/finalization races. Run through `scripts/check-cdk-upstream.py`. |

The compiled local `tests/grpc_contract.rs` uses the exact pinned generated
schema on loopback: Pending with no payment proof, tonic Unavailable, stream
failure, stopped-listener reconnection failure, and a fresh server returning
Pending. This fixture has **no mint database or proof lock implementation**.
Aborting tonic's listener does not guarantee existing connections stop; the test
asserts a new connection fails instead of assuming all accepted sockets close.

Upstream regression tests exercise a real mint with synthetic payment backends
and real proof-state reads. They do not connect our fixture to cdk-mintd, and
their restart test stops/starts the mint over the retained store rather than
restarting a process and reopening a production database. Combined gRPC fault,
durable process restart and custom mpesa/KES/sat mint tests remain a stage 19
gate; source tracing above is the explicit limit of the stage 01 evidence.

## Stage 06 transport boundary

[Payout and balance transport](payout-transport.md) records implemented field
spellings, gated command/identifier candidates, local input policy and remaining
current-contract access limits. The current top-up command BusinessPayToBulk is
not evidence for BusinessTransferFromMMFToUtility. New wire/01/till gates remain
disabled independently of DUP-B2C, DUP-B2B and SWEEP.

## Stage 07 ingress boundary

[Callback ingress](webhook-ingress.md) implements endpoint-specific admission,
STK stored-checkout binding, C2B account/shortcode binding and atomic inbox/job
acknowledgment. Synchronous C2B validation is default-off and requires operator
verification of activation, rejection codes and timeout/default-response behavior.
Held top-ups use the remaining verified allocation under explicit policy. No
callback allocates a receipt or authorizes settlement; external gates stay disabled.

## Stage 08 checkout boundary

[Durable checkout](inbound-checkout.md) adds the shared response model and
server-enforced 35-second retry eligibility without changing pinned CDK or provider
wire contracts. Current STK query/simulation pages exposed no usable terminal
failure-code contract on review; STK-RESEND remains disabled and its failure-code
policy defaults empty. Explicit synthetic test policy proves service behavior only.
Stage 11 supplies the [tariff/FX library](tariffs-fx.md), including conservative
whole-KES/sat `total_spent` mapping. CDK unit advertisement requires explicit
deployed-mint verification and a valid oracle for sat; defaults advertise no units.
No live rate or deployed CDK support is fabricated.

## Stage 12 dispatch boundary

[Durable outbound dispatch](outbound-dispatch.md) adds concrete one-send claims,
frozen endpoint/body/credentials, transactional acknowledgment IDs and staged
callback correlation. No wire contract or provider capability was inferred. All
possibly sent failures remain Pending; duplicate probes and final payout evidence
acquisition remain separately gated work for stage 13 and later operator workflows.

### Outbound reconciliation implementation

The [outbound worker](outbound-reconcile.md) uses only concrete TransactionID
lookups. It never treats a caller/proxy conversation ID as a transaction ID or
enables STATUS-ID implicitly. Same-operation probing additionally requires an
unexpired endpoint/command-specific sandbox/protocol evidence reference and a
verified duplicate-retention age; no approval is configured by default. A literal
duplicate error is Pending. Callback/query success remains an untrusted hint;
independent exact-dispatch evidence with an actual receipt authorizes Paid,
while definitive non-execution evidence is required for Failed. Manual deadlines
produce a durable alert and Pending, preserving the pinned CDK proof-safety mapping.

### Liquidity observations and linked sweeps

[Liquidity](liquidity.md) adds no assumed provider balance layout or authenticity.
BALANCE-SOURCE requires independently verified account/available-KES evidence at
or after query start; its default verifier is unavailable. AccountBalance replies
bind both acknowledged IDs and cannot renew freshness through late delivery.
Optional sweep orchestration preserves the exact gated MMF command, linked
accounts, one-send saga and ambiguity rules. SWEEP, WIRE-MMF and WIRE-B2B-ID remain
disabled, independently of any duplicate-probe guarantee. No live transfer ran.

# Pinned foundation contract

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

Proposed decision for stage 01: target the exact v0.18.2 service and route
incoming notifications through `WaitPaymentEvent.payment_received`; retain outgoing
event semantics for explicit review. Do not rename the RPC or fabricate a legacy
compatibility service. Only type generation is accepted here; event mapping,
proof locks, custom `mpesa`/KES/sat handling and all adapter behavior remain gated
on stage 01 evidence and stages 17–19 implementation.

## Capability boundary

No stage 00 code opens a socket, reads credentials, calls a provider, dispatches
money or settles a quote. Configuration is a documentation template only and all
listed unverified capabilities are false. Stage 01 creates the detailed live gate
register; no duplicate-rejection, regulatory, receipt verification or proof-lock
guarantee has been established by this build.

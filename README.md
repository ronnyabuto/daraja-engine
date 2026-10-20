# daraja-engine

Self-hosted Rust components for integrating Safaricom Daraja with a Cashu mint
through the CDK payment processor protocol.

The repository contains one Cargo workspace:

- [daraja-core](crates/daraja-core): financial domain types, SQLite persistence,
  OAuth and cryptography, and collection/query/payout HTTP clients.
- [cdk-daraja-processor](crates/cdk-daraja-processor): pinned CDK protobuf types
  and a processor library/binary under development.
- [daraja-daemon](crates/daraja-daemon): planned REST/gRPC sidecar.

**Work in progress.** Stages 00–12 are locally verified. Inbound reconciliation is available as a library;
runnable services remain unimplemented; callback ingress is available as a
library router, and durable dual-rail checkout is available as a library service.
Both binaries exit unsuccessfully with an explicit diagnostic. No live provider
or combined mint integration has been verified; all external capabilities default
to disabled. See the [roadmap](docs/roadmap.md) for the remaining work.

## Documentation

- [Financial state machines](docs/state-machines.md)
- [SQLite schema and recovery contracts](docs/schema.md)
- [Credential and cryptography contracts](docs/auth-crypto.md)
- [Collection and query transport](docs/collection-transport.md)
- [Durable inbound checkout](docs/inbound-checkout.md)
- [Inbound reconciliation and notification replay](docs/inbound-reconcile.md)
- [Deposit exceptions, risk holds and gated refunds](docs/deposit-exceptions.md)
- [Tariff schedules and frozen KES/sat valuation](docs/tariffs-fx.md)
- [Durable outbound dispatch](docs/outbound-dispatch.md)
- [Payout and balance transport](docs/payout-transport.md)
- [Callback ingress and C2B validation](docs/webhook-ingress.md)
- [Pinned protocol and provider findings](docs/protocol-contract.md)
- [Verification gates](docs/verification-gates.md)

## Build locally

Install rustup and a native C/C++ build toolchain with a linker (Ubuntu:
`build-essential`, `pkg-config`; macOS: Xcode command-line tools). Python 3.11+
is needed for the foundation policy check. Rustup installs the pinned Rust
1.93.0 toolchain, rustfmt and clippy from rust-toolchain.toml. The first build
needs network access to Rust distribution and Cargo registries; after fetching,
`--offline --locked` works from the dependency cache.

The protobuf compiler is pinned via protoc-bin-vendored 3.2.0 in Cargo.lock;
no system protoc is required. Build on a host supported by that crate; locally
verified here on x86_64 Linux. SQLx builds bundled SQLite using the C compiler;
no database server or DATABASE_URL is needed. Generated Rust/descriptor output
lives only under target/, and builds never download the CDK schema.

```sh
cargo build --workspace --locked
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 scripts/check-foundation.py
python3 scripts/test-protocol-gates.py
```

Cargo.lock is committed for this application workspace. Keep `--locked` in CI.
For a fresh-output build, use `CARGO_TARGET_DIR=/tmp/daraja-clean-build cargo
build --workspace --locked` on one line. See [the protocol contract](docs/protocol-contract.md)
for exact versions, schema provenance and the `WaitPaymentEvent` PRD mismatch.
[Verification gates](docs/verification-gates.md) documents the opt-in sandbox
probe, pinned upstream CDK test runner and disabled external capabilities.

`config.example.toml` contains placeholders only and is not parsed yet. Operators
will own and supply their credentials locally in later stages; populated
`config.toml`, `config.local.toml`, environment files, secrets and key files are
ignored. Do not put credentials in Cargo manifests, source or documentation.

Original workspace code is licensed under either [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option. Vendored CDK notices accompany the
schema. Internal implementation prompts, session handoffs and source specification files
are maintained locally and excluded from the distributed source tree.

# daraja-engine

Implementation workspace prepared from **PRD & Technical Specification v1.0: daraja-core & cdk-daraja-processor**.

The three project folders follow the specification's single Cargo workspace:

- [daraja-core](crates/daraja-core): Rust reconciliation library.
- [cdk-daraja-processor](crates/cdk-daraja-processor): CDK gRPC payment processor.
- [daraja-daemon](crates/daraja-daemon): language-agnostic REST/gRPC sidecar.

**Status: stages 00–01 build foundation and local protocol verification; no payment engine or service is implemented yet.** One Cargo workspace keeps the core, processor and daemon on the same state/protocol contracts. Both binaries exit unsuccessfully with an explicit diagnostic until their service stages are implemented.

Start with [the sequential implementation guide](docs/implementation/README.md). The original [PDF](docs/spec/prd-v1.0.pdf) and [extracted text](docs/spec/prd-v1.0.txt) are retained locally.


GitHub repository: [ronnyabuto/daraja-engine](https://github.com/ronnyabuto/daraja-engine).

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
schema. The original specification and planning documents remain preserved.

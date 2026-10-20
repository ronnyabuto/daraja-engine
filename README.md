# daraja-engine

Implementation workspace prepared from **PRD & Technical Specification v1.0: daraja-core & cdk-daraja-processor**.

The three project folders follow the specification's single Cargo workspace:

- [daraja-core](crates/daraja-core): Rust reconciliation library.
- [cdk-daraja-processor](crates/cdk-daraja-processor): CDK gRPC payment processor.
- [daraja-daemon](crates/daraja-daemon): language-agnostic REST/gRPC sidecar.

**Status: folders and implementation plan only; no payment engine is implemented yet.** Cargo manifests/source will be created by prompt 00. One workspace keeps the core, processor and daemon on the same state/protocol contracts.

Start with [the sequential implementation guide](docs/implementation/README.md). The original [PDF](docs/spec/prd-v1.0.pdf) and [extracted text](docs/spec/prd-v1.0.txt) are retained locally.


GitHub repository: [ronnyabuto/daraja-engine](https://github.com/ronnyabuto/daraja-engine).

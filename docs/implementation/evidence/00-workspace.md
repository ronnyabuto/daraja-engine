# Stage 00 evidence

Date: 2026-10-20. Workspace: `/home/officialnyabuto/Desktop/daraja-engine`.
Branch: `feat/00-workspace`. Base: `f4dc6513cacd206c6b677d5da2af16742073751d`.
Stage changes are initially dirty relative to that base; the commit containing
this evidence is the stage 00 code revision. No push, publication or live payment
was performed. No user changes existed at stage start.

## Prerequisites and scope

Read AGENTS.md, STATUS.md, requirements.md, stage 00, the implementation guide,
crate READMEs and relevant PRD sections 1, 4, 5.1 and 7. Verified PDF wording using
`pdftotext -f 5 -l 8 docs/spec/prd-v1.0.pdf /tmp/daraja-prd-foundation.txt`.
Prepared folders/specification existed; no prior numbered stage was required.
There were no existing source callers, tests or protocol/state contract files.
All provider/CDK financial capabilities remain unverified and unimplemented.

Applied new-feature once: `git fetch origin`, then `git rev-parse origin/main`
returned the base above; `git switch -c feat/00-workspace origin/main` started at
that exact base in the assigned checkout. The repository already had origin/main;
no repository initialization or fallback base was necessary. No agents were spawned.
Applied current-docs to the version-sensitive build/code-generation choices;
sources and exact pins are in `docs/protocol-contract.md`.

## Upstream inspection

```sh
git clone --depth 1 --branch v0.18.2 https://github.com/cashubtc/cdk.git /tmp/daraja-cdk-v0.18.2
git -C /tmp/daraja-cdk-v0.18.2 rev-parse HEAD
sha256sum crates/cdk-daraja-processor/proto/payment_processor.proto
```

Resolved commit: `9f6d3a3734d7223d10b5553e3591d3f9d5c6c904`.
Schema SHA-256: `1b618a8f0bb0ecfc704e3b856697b524843bb944ae6bcde848698c06cd7b37ee`.
Inspected original proto, root/crate Cargo manifests, build.rs and license files;
copied schema unchanged and preserved upstream licenses. `WaitPaymentEvent`
differs from PRD `WaitIncomingPayment`; the exact mismatch and proposal are
recorded in the protocol contract before any adapter implementation.

## Executed local checks

Host: x86_64 Linux; native compiler `cc (Ubuntu 15.2.0-16ubuntu1) 15.2.0`.
Installed stable initially reported `rustc 1.93.0 (254b59607 2026-01-19)` and
`cargo 1.93.0 (083ac5135 2025-12-15)`. No system protoc existed.
Vendored host compiler reports `libprotoc 31.1`.

Rustup installation of the named 1.93.0 toolchain required approved writes outside
the workspace. The initial sandbox attempt failed read-only; the approved retry
downloaded some components but the rustc archive transfer stalled. The redundant
installation was cancelled (exit 130); no installer remains running. All build,
test, formatting and lint checks used `+stable`, explicitly verified to be the
exact pinned Rust/Cargo versions above. The version-named rustup installation
was not completed locally; a fresh machine needs network access for it, as
documented. This is an environment setup limitation, not an upstream schema or
application build blocker: clean-output compilation with the exact compiler passed.

| Command / probe | Actual result |
|---|---|
| `cargo +stable generate-lockfile` | Success; 181 packages locked for Rust 1.93 |
| `cargo +stable build --workspace --locked` | Success; all three crates, processor library + binary |
| `cargo +stable fmt --all -- --check` | Success |
| `cargo +stable test --workspace --locked` | Success; 2 wire-contract tests, no failures |
| `cargo +stable clippy --workspace --all-targets --locked -- -D warnings` | Success |
| Run both `target/debug` binaries with subprocess | Both exit 1 with explicit unavailable diagnostic |
| `python3 scripts/check-foundation.py` | Success; checksum, tracked-file policy, placeholder values, disabled template capabilities |
| `git check-ignore .env config.toml secrets/test.txt example.key target/test` | All five ignored |
| `git diff --check` | Success |

The descriptor test asserts the real package/service and exact seven RPC names,
including the server-streaming event method. The second test round-trips a local
synthetic custom quote through generated protobuf types, retaining amount, unit,
mint quote identity and custom JSON. These do not establish financial correctness.

## Reproducibility

`CARGO_TARGET_DIR=target/clean-stage00 cargo +stable build --workspace --locked --offline`
passed from an initially absent output directory. No schema/tool download occurred.
Compared SHA-256 across every matching file beneath `target/debug/build/` and
`target/clean-stage00/debug/build/` using Python hashlib; both builds agree:

| Generated artifact | SHA-256 |
|---|---|
| cdk_payment_processor.rs | f0cd7a55a862f6e02369f6fa164b6f1e86dda66104091cff5808a077cb8705c4 |
| cdk_payment_processor.bin | f96e4f6e667c809b742c1b4da705a01291baf46b76c644d5768858064f1dfa2d |

Reproduce the comparison after the two builds:

```python
from pathlib import Path
from hashlib import sha256
for name in ["cdk_payment_processor.rs", "cdk_payment_processor.bin"]:
    hashes = []
    for base in ["target/debug/build", "target/clean-stage00/debug/build"]:
        paths = list(Path(base).glob("cdk-daraja-processor-*/out/" + name))
        assert paths
        hashes.extend(sha256(path.read_bytes()).hexdigest() for path in paths)
    assert len(set(hashes)) == 1, hashes
    print(name, hashes[0])
```

Failure/recovery probe: saved original schema bytes, appended a synthetic comment,
ran `cargo +stable build -p cdk-daraja-processor --locked --offline`, and asserted
nonzero exit with `CDK schema checksum differs from the pinned upstream contract`.
Restored the original bytes in Python `finally`, then ran
`cargo +stable build --workspace --locked --offline`: success. The foundation
policy check also rejected the altered checksum during this probe and passed
again after restoration. No synthetic change remains in source.

The build prerequisites and clean build commands are documented in root README.md.
These results establish repeatable generated artifacts on the tested host, not
bit-for-bit application binaries across platforms.

## Limitations and handoff

CI configuration is provided; no hosted CI run or cross-platform build is claimed.
Configuration parsing, financial logic, service startup, CDK mint integration and
live gates belong to later stages. Application code has no provider/network calls.
No live credentials were supplied or added; templates contain placeholders only.
The automated tracked-file check is a path/template policy check, supplemented by
review of all stage additions; it is not a general secret-detection guarantee.

Next task: execute `docs/implementation/prompts/01-protocol-gates.md` in this same
branch/checkout. Resolve the recorded CDK compatibility proposal and establish
the capability gate register before implementing adapter behavior.

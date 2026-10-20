# Stage 01 evidence

Date: 2026-10-20. Checkout: `/home/officialnyabuto/Desktop/daraja-engine`.
Actual branch: `main`; starting HEAD and refreshed origin/main:
`1120bf42166e6c1b4df8b793bf0520a7b4231be4`. Stage 01 changes were **dirty during verification**, before the subsequent
user-authorized commit and push to main. No service was deployed. The tree was clean before this stage.
Stage 00's recorded `feat/00-workspace` did not match the actual checkout; the
actual branch/worktree was preserved without reset/switch. `git fetch origin`
first failed because `.git/FETCH_HEAD` was read-only in the sandbox; its approved
retry succeeded. Applied new-feature once in continuation mode; current-docs and
system-reasoning informed the version/trust/proof-lock investigation. No agents
were spawned.

## Prerequisite and research

Read root AGENTS.md, STATUS.md, requirements.md, implementation guide, stage 01,
SPEC-NOTES.md, existing schema/build/test callers, config and stage 00 evidence.
Stage 00 is locally DONE; full CDK/provider/legal gates were unverified and are
not prerequisites for independent local stage 01 work.

```sh
pdftotext -layout docs/spec/prd-v1.0.pdf /tmp/daraja-prd-stage01.txt
git -C /tmp/daraja-cdk-v0.18.2 rev-parse HEAD
git ls-remote https://github.com/cashubtc/nuts.git HEAD
git ls-remote https://github.com/safaricom/mpesa-java-sdk.git HEAD
git ls-remote https://github.com/safaricom/mpesa-php-sdk.git HEAD
```

Compared PDF sections 2, 4, 5.2, 5.3 and Phase 0 with the supplied extraction;
the listed protocol/legal assertions are present in both. CDK source already
existed under /tmp from stage 00, at exact pin
`9f6d3a3734d7223d10b5553e3591d3f9d5c6c904`, with unchanged tracked source/lock.
NUT pin: `8bde3c0c3684430d852ab543ac8ca72913770dc0`.
Primary SDK pins and primary provider/legal URLs are in the protocol contract
and gate register. Direct Daraja pages often exposed only the shell; indexed
primary-page content supplied current documentation, and inaccessible/current
response details are explicitly unverified. No secondary SDK is treated as a
provider guarantee.

Key findings: exact RPC is WaitPaymentEvent; protocol version/header are 4.0.0 /
x-cdk-protocol-version. Status conversation lookup is documented using
`OriginalConversationID`, distinct from payout `OriginatorConversationID`.
B2C duplicate error is documented but not live-verified; B2B guarantee and caller
ID acceptance remain unverified. Status callback sample has malformed duplicate
keys and no dependable amount attestation. Query correlation supplies no
signature. C2B validation requires separate enablement. STK/C2B callback shapes
differ from the PRD's shared description. Custom units/method types exist in
pinned CDK, but actual mpesa mint integration remains gated.

Legal verification used current official sources, including final CMA-hosted
LN 134 rather than relying only on the March draft. Reproduce the consequential
table check:

```sh
curl -sS -L https://www.cma.or.ke/download/34/regulations/6377/the-virtual-asset-service-providers-regulations-2026.pdf -o /tmp/daraja-vasp-final.pdf
sha256sum /tmp/daraja-vasp-final.pdf
pdftotext -layout /tmp/daraja-vasp-final.pdf /tmp/daraja-vasp-final.txt
pdftoppm -f 115 -l 115 -scale-to 1600 -png -singlefile /tmp/daraja-vasp-final.pdf /tmp/daraja-vasp-capital
```

SHA-256: `8ff0f1f80ea9811122daefabdd832eee0e0ef5a193c20d7cb02e13293e112ee6`.
Visually inspected PDF page 115, Gazette page 4367: processor paid-up KES 10m;
liquid capital 100% of current liabilities for at least 30 days. This contradicts
the PRD's 4m/8% processor liquidity claim. Gazette cover is dated 22 July 2026,
Supplement 185, LN 134. Counsel/operator applicability is still blocked.
CBK's NPS 2026 source is a draft; consultation close is not enactment.

## Executable local checks

Compiler: `cargo +stable`, using the exact Rust 1.93.0 baseline from stage 00.
Direct dependency pins were preserved; Cargo.lock adds only the test crate's
edges to already resolved tokio/tokio-stream. No provider credentials were read
or stored, and no provider request was sent.

| Command | Actual result |
|---|---|
| `cargo +stable test --workspace --locked --offline` | PASS: 3 processor integration tests (2 existing wire tests plus 1 loopback gRPC scenario), all workspace targets/doc tests successful |
| `cargo +stable fmt --all -- --check` | PASS |
| `cargo +stable clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `python3 scripts/test-protocol-gates.py` | PASS: 6 tests; independent B2C/BuyGoods/PayBill body and ID equality; auth/credential/default denial; status spelling/duplicate JSON keys; HTTP request bytes, numeric response redaction and redirect refusal; transport ambiguity/no retry; disabled register/template |
| `python3 scripts/check-foundation.py` | PASS: vendored checksum, tracked-file/template policy, all capabilities false |
| Synthetic offline CLI plan and changed-amount repeat | PASS: offline-plan / LOCAL_ONLY / settlement_authorized=false; changed amount with same ID refused |
| `git diff --check` | PASS |

The synthetic CLI exercise imported `payload()` from test-protocol-gates.py,
wrote temporary JSON, called `probe.main(['b2c','--payload',path])`, then changed
Amount from 10 to 11 and called it with `--repeat-of` the original. Returned 0
and 1 respectively; no execute flag/token/network was used. Equivalent permanent
regression coverage is in the six-test script. The plan hash was
`8b14355b934bba2f8e46487d64610dc119028f6e47f01226961f707aee88b548`.

During development, the loopback check initially assumed listener-task abort
closed accepted connections. It failed because tonic retains serving tasks;
the corrected check tests failed **new** connection establishment and separately
tests stream Unavailable. The fixture's fresh server returns the same synthetic
Pending response but has no proof database. The Python check initially rejected
the four-digit final error-code component; corrected the allowlist and confirmed
500.002.1001 is retained while body/message/URL/token data are omitted.

## Real pinned CDK checks

Initial direct `cargo +stable test --manifest-path /tmp/daraja-cdk-v0.18.2/Cargo.toml
-p cdk --no-default-features --features mint --locked --lib pending_async_melt_tests
-- --test-threads=1` reached compilation but failed because upstream signatory
build needs protoc. The runner now takes an explicit compiler path. Initial
offline runner failed due to uncached dependencies; online retry needed approved
Cargo-cache writes after a read-only cache error. The approved run uses upstream
Cargo.lock and untouched tracked source, without repinning this workspace.

```sh
python3 scripts/check-cdk-upstream.py --source /tmp/daraja-cdk-v0.18.2 --protoc /home/officialnyabuto/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/protoc-bin-vendored-linux-x86_64-3.2.0/bin/protoc
```

Runner executes `cashu --no-default-features --features mint --lib
nuts::nut00::tests`, then `cdk --no-default-features --features mint --lib` with
`pending_async_melt_tests` and `mint::saga_recovery::tests`, always `--locked`
and `--test-threads=1`, build jobs 2. Upstream resolves tonic 0.14.6 / prost
0.14.4 / Tokio 1.53.2 in its own lock; local generated service retains workspace
0.14.2 / 0.14.1 / 1.48.0 pins. Protocol identity does not establish complete
cross-version integration.

- Cashu type suite: **31 passed**, including custom currency normalization,
  custom method names and currency derivation behavior.
- Real mint async-melt suite: **14 passed**, including Pending->terminal
  compensation, local-looking backend error->Pending with proofs still locked,
  Unknown check preserving locks, successful event, timeout preserving locks,
  lookup identity persistence and startup stalled-check recovery. Runtime 212.86s.
- Saga recovery suite: **12 passed**, including failure/finalization races,
  stale pending outcomes, authoritative terminal compensation, unknown outcomes
  preserving state and no second backend check during compensation. Runtime 49.08s.
  Runner exit status: 0; upstream tracked source/lockfile still clean.

These mint tests read actual proof states and stored sagas using synthetic
backends; they are stronger than a hand-written proof-state simulation. They do
not pass external gates or replace stage 19's combined gRPC/client/mint and
durable process-kill/reopen checks. Error/disconnect source paths where that
combined harness does not yet exist are recorded in protocol-contract.md.

## Handoff boundary

Stage 01 is DONE for local acceptance (including the explicitly permitted source
trace for the combined harness not yet available). All provider
gates remain disabled/BLOCKED separately from local stage acceptance: live
credentials/authorization, endpoint-specific duplicate semantics, query
authenticity and receipt/amount attestation, validation activation, actual mpesa
mint integration, operator/counsel approval. No live trial or publication occurred.
Next numbered task after local completion: prompt 02, state models and accounting
invariants, in this same checkout. Do not restart stage 00 or enable a gate from
this local evidence.

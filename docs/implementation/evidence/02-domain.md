# Stage 02 — domain acceptance

Date: 2026-10-20. Local acceptance: **DONE**. External capabilities: unchanged,
disabled; live evidence acquisition remains **BLOCKED** as recorded in
`docs/verification-gates.{md,json}`. No live requests, payouts or deployment.
After local acceptance, the user authorized committing and pushing this stage to main.

## Checkout, prerequisite and source evidence

- Assigned checkout `/home/officialnyabuto/Desktop/daraja-engine`, preserved `main`.
  Initial `git status --short --branch` was clean. `git worktree list` showed only
  this assigned checkout. `git fetch origin` succeeded; `git rev-parse HEAD
  origin/main` returned `57615088399d61954623d6f6d070722b40852fae` for both.
- Stage 01 STATUS and its evidence were DONE locally; no external gate was
  substituted with local evidence. Read root instructions, requirements,
  stage 02/03 prompts, PRD §§4–5.3, pinned protocol contract, capability JSON,
  existing core manifest/source, processor/daemon callers and existing tests.
  Core was an empty owner crate with no financial callers/tests to preserve.
- `pdftotext -layout docs/spec/prd-v1.0.pdf /tmp/daraja-prd-stage02.txt` followed
  by `cmp docs/spec/prd-v1.0.txt /tmp/daraja-prd-stage02.txt` both exited 0.
- Applied installed new-feature once for the recorded continuation, preserving
  branch/worktree, and system-reasoning for the accounting invariants. No agents
  were spawned. No uncertain version-specific integration required current-docs.
- Revision during verification: **57615088399d61954623d6f6d070722b40852fae + dirty
  stage 02 changes** (uncommitted). Existing pinned sha2 0.10.9 is now a core
  dependency; Cargo.lock only adds that core dependency edge, no version changes.

## Implemented contracts

- `crates/daraja-core/src/amount.rs`: checked explicit KES minor/sat amounts,
  exact whole-KES conversion, frozen tariff and rational floor-rounded FX terms.
- `identity.rs`: redacted opaque IDs, normalized receipt identity, scoped
  idempotency keys, canonical SHA-256 payload hash, immutable replay binding and
  separate caller/proxy/provider correlation.
- `inbound.rs`: distinct quote/attempt/deposit states, multiple STK attempts,
  separate untrusted observations and independently attested value, single
  receipt allocation, top-up accounting, exception custody, hold/expiry checks,
  and a single aggregate mutation for settlement plus event insertion.
- `outbound.rs`: immutable dispatch data, fee-inclusive reservation accounting,
  guarded terminal resolution and Pending projection for every ambiguous/manual
  state. No retry transport or unverified capability is enabled.
- `docs/state-machines.md`: evidence prerequisites, transition tables, canonical
  encoding, timing/rounding policies, trust boundary and persistence obligations.

## Actual commands and results

Executed from the assigned workspace:

| Command | Result |
|---|---|
| `cargo check -p daraja-core --offline` | PASS after toolchain bootstrap recovery |
| `cargo test --workspace --locked` | PASS: 23 domain tests + 1 gRPC contract + 2 wire contract tests; no failures; doc tests pass |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `python3 scripts/check-foundation.py` | PASS: proto checksum, tracked-file policy and config placeholders |
| `python3 scripts/test-protocol-gates.py` | PASS: 6 tests; no live access |
| `git diff --check` | PASS |

The domain suite includes 44,550 fee/FX cases over amounts 1–99, all lower fees,
and denominators 1–9. It checks floor conversion cannot create value, exact quote
credit, zero-credit rejection, overflow, underflow and currency mismatch. Focused
cases also check quote/account uniqueness, immutable replay identity, all hashed
fields, receipt case normalization, duplicate/cross-quote receipts, spoof-first
recovery, exact top-ups, no partial mutations on rejected evidence/overflow,
hold/expiry boundaries, multiple STK attempts and terminal regressions.

Rail tests exercise both arrivals before settlement (overpayment held) and
competing threads each confirming/settling under aggregate exclusive access
(one credit, second receipt in exception custody). Payout tests cover all six
transport/recovery ambiguity reasons, duplicate observation, immutable dispatch
data, manual deadline/escalation, conflicting correlation, mismatched final
evidence and every nonterminal-to-terminal resolution. Both terminal states
reject further transitions. Sensitive options and wire bytes are redacted in Debug.

Initial failures and recovery: concurrent first-use rustup invocations raced
while downloading the missing pinned 1.93.0 compiler (one download rename failed).
A subsequent invocation installed the pinned toolchain and compiled successfully.
Initial property fixtures incorrectly assumed floor conversion always yields
positive credit; the test now explicitly verifies zero-credit quote rejection.
The canonical receipt type rejected a hyphenated synthetic receipt; that fixture
was changed to alphanumeric and an explicit invalid-spelling test retained.
The final workspace test command above passed all 26 integration tests. No
financial invariant, existing test or capability gate was weakened.

## Limits and next stage

`Accounting` is volatile domain state. Its all-or-nothing validation/mutation and
single settlement event are tested locally; this is **not evidence of durable
outbox insertion, database isolation or crash recovery**. Trusted evidence types
validate bindings but do not authenticate a provider source. Synthetic evidence
must never become a production settlement fallback. Refunds/re-quotes, tariff/FX
acquisition, liquidity admission, real provider verification and CDK adapters
remain their numbered stages. Live settlement/duplicate-probe/CDK/legal gates
remain disabled; no missing access blocks completion of this local domain stage.

R08 is locally DONE. R09, R10 and R11 remain IN_PROGRESS across their later
stages. Next task: execute `docs/implementation/prompts/03-sqlite.md` in this
same checkout; implement and verify the durable transaction/CAS boundary using
file-backed SQLite, including rollback, competing writers and reopen/recovery.

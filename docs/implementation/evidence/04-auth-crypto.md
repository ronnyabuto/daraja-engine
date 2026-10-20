# Stage 04 — Credential isolation and cryptography

Date: 2026-10-20. Requirement: R12. State: DONE locally.

## Prerequisites and checkout

Read root AGENTS.md, STATUS.md, requirements.md, prompt 04, relevant PRD sections
4/5.1/phase 1 and protocol/state contracts. Re-extracted the authoritative PDF with
`pdftotext -layout docs/spec/prd-v1.0.pdf /tmp/daraja-prd-04.txt` and inspected its
credential isolation/workspace requirements. No existing auth/crypto callers or
implementations existed. Stage 03 was recorded DONE and its full persistence suite
passed again below. No financial state or persistence code changed.

Applied installed new-feature once, preserving the assigned main/worktree. Ran:

```sh
git status --short --branch
git worktree list
git rev-parse HEAD
git fetch origin
git rev-parse origin/main
```

Initial checkout clean. Both revisions were
`84e9fdc8a21b90d1baf67024fb7b397557822ba8`. Verified code is **dirty/uncommitted stage
04 work on that revision**, including new auth/crypto modules, tests and fixtures,
Cargo manifest/lock changes, configuration template and docs. No branch changes,
commit, push or subagents. Current-docs resolved the consequential API questions
against provider documentation and exact installed Rust crate source; links and
ambiguity handling are recorded in [auth/crypto contracts](../../auth-crypto.md).
Existing dependency versions were preserved; the lockfile adds the new HTTP,
certificate, time and tracing dependency graph and disambiguates shared versions.

## Implemented boundaries

- `src/auth.rs`: typed CollectionApp/DisbursementApp actors with independent
  credentials/clients/cache/refresh mutex/cancellation, shared only by clones of
  the same actor. First-use initialization and idle background proactive refresh.
  A one-hour token refreshes at 3000 seconds; shorter lifetimes retain a one-sixth
  margin. `expires_in` accepts integer/string seconds and never defaults to an hour.
  Network latency is counted against expiry. Held snapshots refuse expired headers.
- One in-flight refresh per actor, double-checked cache and short RwLock sections.
  Errors are shared during a five-second backoff; old tokens are discarded on
  refresh. Full-request 15-second deadline, 16 KiB successful-body bound, no redirects
  or HTTP retries. Explicit cancellation and last-owner drop stop background work.
- AuthorizationAttempt permits one generation-aware refresh after a 401, including
  concurrent and stale 401s. A failed attempt loses its token. No financial HTTP
  request/replay is performed or authorized by this module.
- `src/crypto.rs`: redacted, zeroizing Secret values; injectable UTC clock -> EAT
  timestamp and Base64 STK password; explicit environment certificate paths;
  DER/PEM X.509 -> RSA SPKI extraction; OS-randomized PKCS#1 v1.5 encryption of
  original plaintext password bytes -> Base64. Environment mismatch fails closed.
- Config/token/attempt/crypto Debug and errors omit secrets. Sensitive HTTP headers
  plus a captured TRACE test exercise real request/response and error boundaries.

## Reproducible checks and actual results

All commands executed locally with the repository's Rust/Cargo 1.93.0 toolchain:

```sh
cargo fetch
cargo check -p daraja-core --locked
cargo test -p daraja-core --test auth --test crypto --locked
cargo test -p daraja-core --test redaction --locked
cargo test -p daraja-core --test auth --test crypto --test redaction --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-foundation.py
python3 scripts/test-protocol-gates.py
git diff --check
```

- Fetch/check succeeded. Removing unused reqwest json/stream features refreshed
  the lockfile with `cargo check -p daraja-core`; all final commands used `--locked`.
- Focused final tests: **12 OAuth, 3 crypto, 1 redaction passed**. Initial focused
  suites also passed; subsequent API hardening added held-token/delayed-response
  expiry coverage and removes rejected tokens from failed authorization attempts.
- Workspace: **60 top-level test entries passed**, including 12 OAuth, 3 crypto,
  1 TRACE redaction, 23 domain, 17 SQLite integration entries (two child helpers),
  1 connection-settings test and 3 protocol integration tests. Existing subprocess
  SQLite crash/writer tests and all doctest targets passed.
- Clippy workspace/all-targets warnings denied: passed.
- Formatting: passed. Foundation tracked-file/config/proto checks: passed.
  Python protocol probe tests: **6 passed**. Whitespace: passed.
- Workspace and Clippy logs were captured as `/tmp/daraja-04-workspace-tests.log`
  and `/tmp/daraja-04-clippy.log`; summaries above are retained here for handoff.

OAuth tests run an ephemeral loopback HTTP provider and verify Basic credentials
arrive at the right app path. Tests include 40 concurrent initial acquisitions,
30 concurrent/stale 401 refreshes, three multithreaded boundary tests, proactive
idle refresh at 50 minutes, metadata-driven short lifetimes, expiration of held
snapshots, latency consuming token lifetime, shared refresh rejection, recovery,
request timeout/recovery, request abort/recovery, actor/clone/drop cancellation,
no redirect credential forwarding, malformed/missing/overflow expiry, malformed
bearer tokens and oversized response rejection. Tokio time control accelerates
expiry checks; HTTP requests still traverse real loopback sockets.

Crypto tests use fixed UTC/EAT timestamp/password vectors including year rollover.
Committed synthetic 2048-bit RSA X.509 fixtures in PEM/DER are encrypted twice,
Base64-decoded and decrypted with the matching local private key; each ciphertext
recovers the original password and the two ciphertexts differ. Tests check invalid,
truncated and private-key-instead-of-certificate inputs, non-RSA certificates,
missing files, environment mismatch and oversized plaintext. Fixture generation
commands are in `crates/daraja-core/tests/fixtures/README.md`. No provider material
is committed; `.txt` key fixtures are deliberately public synthetic test data.

The isolated redaction test captures TRACE output from success and provider-error
HTTP paths and Debug/Display rendering of config, actor, token, sensitive header,
authorization attempt, passkey, STK password and encrypted SecurityCredential.
Assertions reject plaintext secret markers, Basic-encoded credentials, bearer
values, provider error-body secrets and generated encoded password/credential values.

## Limits and next stage

All capability gates in docs/verification-gates.json and config.example.toml remain
false; LIVE/legal/provider activation and settlement/idempotency gates remain
BLOCKED or unverified externally. No live OAuth trial, provider certificate
acceptance, money movement, deployment or publishing occurred. Local fixtures
establish protocol and crypto behavior only, not provider permissions or guarantees.

Configured certificate provenance and environment binding require authenticated
operator downloads. Certificate parsing intentionally does not attest provider
identity, validity periods, chain or revocation. The library encrypts with a public
RSA key; private-key operations occur only in synthetic decryption tests. Secret
storage zeroization does not erase copies created in HTTP libraries or by callers.

Next: [Prompt 05 — collection transport](../prompts/05-collection-transport.md).
Consume the auth/crypto contracts, preserve this checkout/branch and dirty work,
keep external gates disabled, and implement only that numbered stage when requested.

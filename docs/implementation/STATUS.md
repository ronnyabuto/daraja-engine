# Implementation status

Last updated: 2026-10-20
Active model requested: gpt-6.1-sol
Reasoning requested: medium
Workspace: /home/officialnyabuto/Desktop/daraja-engine
Assigned Git branch/worktree: actual main at /home/officialnyabuto/Desktop/daraja-engine; preserved. Stage 00 recorded feat/00-workspace, but stage 01 found main at 1120bf42166e6c1b4df8b793bf0520a7b4231be4 with a clean tree. Successful origin fetch confirmed origin/main at the same commit. No branch reset or switch was performed.
Current stage: 04
Implementation state: DONE (stage 04 locally verified; external gates remain disabled)

Stages 00–04 are locally verified. Stage 04 adds isolated cancellable OAuth actors with metadata-driven background refresh, EAT STK passwords and environment-tagged X.509 RSA PKCS#1 v1.5 credentials. Stage 03 adds file-backed SQLx SQLite WAL persistence, ordered migrations, transactional receipt settlement/events, CAS, immutable dispatch preparation and leased recovery work. Stage 02 adds exact financial domain types, guarded transitions, receipt allocation and settlement/event contracts with focused invariant tests. Stage 01 adds protocol research, a compiled loopback gRPC harness, an opt-in sandbox probe, fail-closed capability register and real pinned CDK proof-state regression execution. No live provider trial or combined Daraja/cdk-mintd integration has run.

| Stage | State | Evidence |
|---|---|---|
| 00 workspace | DONE | [00-workspace.md](evidence/00-workspace.md) |
| 01 protocol-gates | DONE | [01-protocol-gates.md](evidence/01-protocol-gates.md) |
| 02 domain | DONE | [02-domain.md](evidence/02-domain.md) |
| 03 sqlite | DONE | [03-sqlite.md](evidence/03-sqlite.md) |
| 04 auth-crypto | DONE | [04-auth-crypto.md](evidence/04-auth-crypto.md) |
| 05 collection-transport | NOT_STARTED | — |
| 06 payout-transport | NOT_STARTED | — |
| 07 webhook-ingress | NOT_STARTED | — |
| 08 inbound-checkout | NOT_STARTED | — |
| 09 inbound-reconcile | NOT_STARTED | — |
| 10 deposit-exceptions | NOT_STARTED | — |
| 11 tariffs-fx | NOT_STARTED | — |
| 12 outbound-dispatch | NOT_STARTED | — |
| 13 outbound-reconcile | NOT_STARTED | — |
| 14 liquidity | NOT_STARTED | — |
| 15 operations | NOT_STARTED | — |
| 16 postgres | NOT_STARTED | — |
| 17 cdk-incoming | NOT_STARTED | — |
| 18 cdk-outgoing | NOT_STARTED | — |
| 19 cdk-end-to-end | NOT_STARTED | — |
| 20 daemon-rest | NOT_STARTED | — |
| 21 daemon-grpc | NOT_STARTED | — |
| 22 reference-cli | NOT_STARTED | — |
| 23 reference-web | NOT_STARTED | — |
| 24 polyglot | NOT_STARTED | — |
| 25 fault-audit | NOT_STARTED | — |
| 26 packaging | NOT_STARTED | — |
| 27 live-verification | NOT_STARTED | — |
| 28 final-closure | NOT_STARTED | — |

## Stage 04 handoff

- Preserved assigned main and checkout, initially clean. Applied new-feature once for this continuation; successful `git fetch origin`, HEAD and origin/main both `84e9fdc8a21b90d1baf67024fb7b397557822ba8`. No branch switch/reset, delegation, commit, push, publishing or live provider requests. Verification used dirty/uncommitted stage 04 changes on that revision, including new files.
- R12 is DONE locally: daraja-core src/auth.rs and src/crypto.rs; independent typed app actors, lazy initialization plus background proactive refresh, metadata expiry, single-flight coordination, five-second shared failure backoff, cancellation/drop recovery and bounded generation-aware 401 refresh. No financial request is replayed. Injectable EAT STK password generation and environment-specific DER/PEM X.509 RSA public-key extraction/PKCS#1 v1.5/Base64 encryption are implemented. docs/auth-crypto.md records the caller and certificate trust contracts.
- Final checks: `cargo test --workspace --locked` passed 60 top-level test entries (including SQLite child helpers), with 12 OAuth + 3 crypto + 1 TRACE redaction tests added; fmt, workspace/all-target Clippy with warnings denied, foundation check, 6 Python probe tests and whitespace check passed. All fixtures are synthetic. See evidence/04-auth-crypto.md for exact commands/results and evidence limits.
- Certificate authenticity/environment provenance is the operator's responsibility; parsing is not provider certificate verification. Real OAuth credentials/certificates and provider acceptance remain external gates. All capability flags remain disabled; stage 04 does not establish payout replay or settlement guarantees.
- Next task: execute docs/implementation/prompts/05-collection-transport.md in this same checkout, preserving branch/worktree and pending stage 04 changes. Do not repeat branch setup or advance beyond that numbered stage.
- The user subsequently authorized committing and pushing stage 04 to main. The stage 04 code revision is the commit containing this handoff; local verification above preceded that commit. No deployment or live money movement was authorized.

## Stage 03 handoff

- Preserved assigned main and checkout, initially clean. New-feature continuation check: successful escalated `git fetch origin`; HEAD and origin/main both `fe468cb9e8556cde1294b91f2db15cd800879a13`. No switch/reset, delegation, commit or push. Verification used dirty/uncommitted stage 03 changes on that revision, including new files.
- Added two SQLx migrations and daraja-core repository; docs/schema.md records normalized identities plus private snapshots, WAL/FULL/foreign-key/busy settings, short transactions, CAS, receipt verification/allocation, exception custody, atomic event outbox, immutable payout/idempotency preparation, balance reservation/audit and lease fencing. No provider I/O or automatic resend.
- Final checks: workspace tests passed (23 domain + 17 SQLite integration entries, including 2 child entry helpers, + 1 connection-settings test + 3 protocol integration tests); fmt, workspace/all-target Clippy with warnings denied, foundation check, 6 probe tests and whitespace check passed. File-backed evidence includes competing processes, migration upgrade/checksum rejection, injected settlement rollback, abrupt exit before dispatch and uncommitted-write recovery. See evidence/03-sqlite.md.
- R09/R10 are DONE locally. R11 remains IN_PROGRESS only for later provider workers and full saga recovery; its stage 03 persistence primitives are locally verified. No external gates changed; SETTLE, duplicate probes, LIVE and other provider/CDK/legal gates remain disabled or BLOCKED externally.
- Next task: execute docs/implementation/prompts/04-auth-crypto.md in this same checkout. Preserve branch/worktree and pending changes. Do not repeat branch setup or advance beyond that one numbered stage.
- The user subsequently authorized committing and pushing stage 03 to main. The stage 03 code revision is the commit containing this handoff; local verification above preceded that commit. No deployment or live money movement was authorized.

## Stage 02 handoff

- Preserved main and the assigned checkout. Stage 02 `git fetch origin` succeeded; HEAD and origin/main both resolved to 57615088399d61954623d6f6d070722b40852fae before edits. Initial tree was clean. Applied new-feature for this continuation without switching/resetting the recorded branch; system-reasoning established accounting boundaries. No delegation.
- Added daraja-core amount, identity, inbound and outbound modules; docs/state-machines.md records transition/evidence tables and the stage 03 transaction contract. Canonical SHA-256 payload identities use the existing pinned sha2 dependency; no dependency versions changed.
- Local checks: `cargo test --workspace --locked` passed 23 domain tests plus 3 existing protocol integration tests; `cargo fmt --all -- --check`, workspace/all-target Clippy with warnings denied, foundation check, 6 probe tests and `git diff --check` passed. See evidence/02-domain.md for exact results and initial fixture failures.
- Stage 02 was verified as dirty changes on the revision above; its code revision is the commit containing this handoff. The user subsequently authorized committing and pushing to main. No deployment or live money movement was authorized. All provider/CDK/legal capability gates are unchanged and disabled.
- R08 is DONE locally. R09/R10/R11 remain IN_PROGRESS: domain identity/allocation and event contracts exist, but SQL uniqueness/CAS, durable jobs/outbox, crash/reopen and cross-process writer tests belong to stage 03 and later stages.
- Next task: execute docs/implementation/prompts/03-sqlite.md in this same checkout. Preserve these changes and branch; do not repeat new-feature setup or treat synthetic attestations as provider verification.

## Stage 01 handoff (historical)

- Completed stage 01: docs/protocol-contract.md and docs/verification-gates.{md,json}; opt-in sandbox-probe.py; test-protocol-gates.py; check-cdk-upstream.py; processor tests/grpc_contract.rs; disabled template capabilities and CI coverage.
- Code revision: the commit containing this stage 01 handoff, based on 1120bf42166e6c1b4df8b793bf0520a7b4231be4; changes were dirty during local verification. Preserve actual main and this checkout. Git push to main was subsequently authorized; no deployment or money movement was authorized.
- Local checks: workspace tests (3 integration tests), fmt, Clippy warnings denied, foundation check, 6 Python probe tests and diff whitespace check passed. Exact compiler remains Rust/Cargo 1.93.0. Real pinned upstream CDK run passed 31 Cashu type tests, 14 mint async-melt tests and 12 saga recovery tests. See evidence/01-protocol-gates.md for commands, initial failures/recovery and evidence limits.
- Accepted exact CDK WaitPaymentEvent RPC, protocol version 4.0.0, x-cdk-protocol-version header. Do not implement the PRD's nonexistent WaitIncomingPayment RPC.
- Provider gates: documented B2C v3 duplicates do not establish live guarantees; B2B caller-ID/duplicates remain unverified. Both independent endpoint gates are required before automatic resubmission. Duplicate rejection is never Paid. Query request OriginalConversationID differs from payout OriginatorConversationID; exact accepted identity mapping remains gated.
- Settlement gate: asynchronous query correlation is not authentication, and success does not attest omitted receipt/amount fields. C2B validation needs separate provider activation. Unverified receipts must not poison legitimate allocation. Timers/manual escalation preserve possibly dispatched payouts as Pending.
- Legal gate: obtained final CMA-hosted LN 134/2026; Fifth Schedule contradicts the PRD payment-processor liquidity formula. NPS 2026 Bill/policy remain draft sources. Operator/counsel classification and provider onboarding remain required, not passed by research.
- CDK limits: actual upstream proof-state regressions are executed, but combined real processor/client/mint faults, durable process-kill/reopen and mpesa/KES/sat route/keyset/FX integration remain disabled until stages 17–19. The loopback fixture has no proof database; precise source traces cover the unexecuted boundary.
- Requirements R03–R07 have stage 01 implementation/test references and remain IN_PROGRESS across later stages; local DONE does not close external gates.
- Stage 01 next action was prompt 02; its local domain acceptance is now complete. Current next task is prompt 03 above.

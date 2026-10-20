# Implementation status

Last updated: 2026-10-20
Active model requested: gpt-6.1-sol
Reasoning requested: medium
Workspace: /home/officialnyabuto/Desktop/daraja-engine
Assigned Git branch/worktree: actual main at /home/officialnyabuto/Desktop/daraja-engine; preserved. Stage 00 recorded feat/00-workspace, but stage 01 found main at 1120bf42166e6c1b4df8b793bf0520a7b4231be4 with a clean tree. Successful origin fetch confirmed origin/main at the same commit. No branch reset or switch was performed.
Current stage: 01
Implementation state: DONE (stage 01 locally verified; external gates remain disabled)

Stages 00–01 are locally verified. Stage 01 adds protocol research, a compiled loopback gRPC harness, an opt-in sandbox probe, fail-closed capability register and real pinned CDK proof-state regression execution. No live provider trial or combined Daraja/cdk-mintd integration has run.

| Stage | State | Evidence |
|---|---|---|
| 00 workspace | DONE | [00-workspace.md](evidence/00-workspace.md) |
| 01 protocol-gates | DONE | [01-protocol-gates.md](evidence/01-protocol-gates.md) |
| 02 domain | NOT_STARTED | — |
| 03 sqlite | NOT_STARTED | — |
| 04 auth-crypto | NOT_STARTED | — |
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

## Handoff

- Completed stage 01: docs/protocol-contract.md and docs/verification-gates.{md,json}; opt-in sandbox-probe.py; test-protocol-gates.py; check-cdk-upstream.py; processor tests/grpc_contract.rs; disabled template capabilities and CI coverage.
- Code revision: the commit containing this stage 01 handoff, based on 1120bf42166e6c1b4df8b793bf0520a7b4231be4; changes were dirty during local verification. Preserve actual main and this checkout. Git push to main was subsequently authorized; no deployment or money movement was authorized.
- Local checks: workspace tests (3 integration tests), fmt, Clippy warnings denied, foundation check, 6 Python probe tests and diff whitespace check passed. Exact compiler remains Rust/Cargo 1.93.0. Real pinned upstream CDK run passed 31 Cashu type tests, 14 mint async-melt tests and 12 saga recovery tests. See evidence/01-protocol-gates.md for commands, initial failures/recovery and evidence limits.
- Accepted exact CDK WaitPaymentEvent RPC, protocol version 4.0.0, x-cdk-protocol-version header. Do not implement the PRD's nonexistent WaitIncomingPayment RPC.
- Provider gates: documented B2C v3 duplicates do not establish live guarantees; B2B caller-ID/duplicates remain unverified. Both independent endpoint gates are required before automatic resubmission. Duplicate rejection is never Paid. Query request OriginalConversationID differs from payout OriginatorConversationID; exact accepted identity mapping remains gated.
- Settlement gate: asynchronous query correlation is not authentication, and success does not attest omitted receipt/amount fields. C2B validation needs separate provider activation. Unverified receipts must not poison legitimate allocation. Timers/manual escalation preserve possibly dispatched payouts as Pending.
- Legal gate: obtained final CMA-hosted LN 134/2026; Fifth Schedule contradicts the PRD payment-processor liquidity formula. NPS 2026 Bill/policy remain draft sources. Operator/counsel classification and provider onboarding remain required, not passed by research.
- CDK limits: actual upstream proof-state regressions are executed, but combined real processor/client/mint faults, durable process-kill/reopen and mpesa/KES/sat route/keyset/FX integration remain disabled until stages 17–19. The loopback fixture has no proof database; precise source traces cover the unexecuted boundary.
- Requirements R03–R07 have stage 01 implementation/test references and remain IN_PROGRESS across later stages; local DONE does not close external gates.
- Next action: execute docs/implementation/prompts/02-domain.md in this same checkout. Do not repeat new-feature setup for the recorded continuation or enable external capabilities from mock/local evidence.

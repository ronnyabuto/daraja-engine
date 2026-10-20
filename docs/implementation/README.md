# Sequential implementation guide

Use **gpt-6.1-sol** with **medium** reasoning in Codex. Set both in the app/session settings before starting; wording inside a prompt cannot select the active model. Keep the assigned workspace/checkout open.

These stages are a task-design choice for this specification, not a measured hard limit on the model. Smaller ownership boundaries, explicit invariants, persistent handoffs and executed acceptance checks reduce omissions and context drift. Neither a model choice nor these prompts guarantees correctness. OpenAI documents medium as a supported/default effort for GPT-6.1 Sol; its family prompting guidance recommends concrete follow-through and proportionate verification, and says to evaluate guidance on the chosen model/workload. Sources: [model guidance](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-6.1-sol), [model page](https://developers.openai.com/api/docs/models/gpt-6.1-sol). Accessed 2026-10-20. No token quota, task-duration limit or account availability is assumed.

## Running the sequence

1. Open /home/officialnyabuto/Desktop/daraja-engine in Codex and select the model/effort.
2. Paste the full contents of [prompt 00](prompts/00-workspace.md), or send the starter prompt below so Codex reads it itself.
3. Require the acceptance evidence and STATUS.md update before advancing. Run prompts in numeric order, one per session/task. A fresh session between stages is recommended; disk handoff carries context.
4. If IN_PROGRESS, resume that stage. If a local prerequisite is BLOCKED, repair it first. An external provider/legal gate may remain blocked while independent local implementation continues with that capability disabled.
5. Do not run the full sequence in one giant request. Do not restart a completed stage or repin upstream dependencies casually.
6. Finish with prompt 28's requirement audit. Local completion, live verification, production readiness and publication are separate claims.

Starter prompt:

```text
Work in /home/officialnyabuto/Desktop/daraja-engine. Read AGENTS.md and
docs/implementation/README.md, then execute the complete instructions in
docs/implementation/prompts/00-workspace.md. Implement this stage, verify its
acceptance criteria and update the disk handoff. Stop at the stage boundary.
```

For the next stage, change only the numbered filename to the next row below. Each prompt contains its own execution contract; you do not need to paste a master prompt first. If Codex needs more than one session, use [RESUME.md](RESUME.md).

## Stage order

| Prompt | Scope | Requirement IDs |
|---|---|---|
| [00: Workspace and pinned build foundation](prompts/00-workspace.md) | Workspace and pinned build foundation | R01 R02 R03 |
| [01: Protocol research and executable verification gates](prompts/01-protocol-gates.md) | Protocol research and executable verification gates | R03 R04 R05 R06 R07 |
| [02: State models and accounting invariants](prompts/02-domain.md) | State models and accounting invariants | R08 R09 R10 R11 |
| [03: SQLite migrations and transactional repository](prompts/03-sqlite.md) | SQLite migrations and transactional repository | R09 R10 R11 |
| [04: Credential isolation and cryptography](prompts/04-auth-crypto.md) | Credential isolation and cryptography | R12 |
| [05: Daraja collection and query clients](prompts/05-collection-transport.md) | Daraja collection and query clients | R13 R14 R15 |
| [06: B2C, B2B and balance wire adapters](prompts/06-payout-transport.md) | B2C, B2B and balance wire adapters | R16 R17 R18 R19 |
| [07: Secure callback ingress and C2B validation](prompts/07-webhook-ingress.md) | Secure callback ingress and C2B validation | R20 R21 |
| [08: Durable inbound quotes and dual-rail checkout](prompts/08-inbound-checkout.md) | Durable inbound quotes and dual-rail checkout | R22 R23 |
| [09: Verified inbound settlement and durable notifications](prompts/09-inbound-reconcile.md) | Verified inbound settlement and durable notifications | R24 R25 R26 |
| [10: Missing-money recovery, top-ups and refunds](prompts/10-deposit-exceptions.md) | Missing-money recovery, top-ups and refunds | R27 R28 R29 R30 |
| [11: Tariff schedules and expiring KES/sat valuation](prompts/11-tariffs-fx.md) | Tariff schedules and expiring KES/sat valuation | R31 R32 |
| [12: Idempotent payouts and durable dispatch saga](prompts/12-outbound-dispatch.md) | Idempotent payouts and durable dispatch saga | R33 R34 |
| [13: Ambiguity resolution and safe manual escalation](prompts/13-outbound-reconcile.md) | Ambiguity resolution and safe manual escalation | R35 R36 |
| [14: Balance telemetry, reservations and optional sweeps](prompts/14-liquidity.md) | Balance telemetry, reservations and optional sweeps | R37 R38 R39 |
| [15: Operator reconciliation and auditable recovery](prompts/15-operations.md) | Operator reconciliation and auditable recovery | R28 R30 R36 R40 |
| [16: Optional PostgreSQL parity](prompts/16-postgres.md) | Optional PostgreSQL parity | R41 |
| [17: CDK settings, create/check incoming and stream](prompts/17-cdk-incoming.md) | CDK settings, create/check incoming and stream | R03 R42 |
| [18: CDK quote/pay/check outgoing and proof safety](prompts/18-cdk-outgoing.md) | CDK quote/pay/check outgoing and proof safety | R43 R44 |
| [19: Pinned cdk-mintd proof-lock integration](prompts/19-cdk-end-to-end.md) | Pinned cdk-mintd proof-lock integration | R04 R05 R44 R45 |
| [20: Sidecar REST/OpenAPI and authenticated service lifecycle](prompts/20-daemon-rest.md) | Sidecar REST/OpenAPI and authenticated service lifecycle | R46 |
| [21: Sidecar gRPC and transport parity](prompts/21-daemon-grpc.md) | Sidecar gRPC and transport parity | R47 |
| [22: Reference Cashu/mpesa CLI wallet](prompts/22-reference-cli.md) | Reference Cashu/mpesa CLI wallet | R48 |
| [23: Dual-rail reference checkout web client](prompts/23-reference-web.md) | Dual-rail reference checkout web client | R49 |
| [24: Polyglot examples and integration AGENTS.md](prompts/24-polyglot.md) | Polyglot examples and integration AGENTS.md | R50 R51 |
| [25: Security, race and crash recovery closure](prompts/25-fault-audit.md) | Security, race and crash recovery closure | R52 |
| [26: Reproducible releases and operator deployment](prompts/26-packaging.md) | Reproducible releases and operator deployment | R53 R54 |
| [27: Sandbox validation and operator readiness gates](prompts/27-live-verification.md) | Sandbox validation and operator readiness gates | R06 R07 R55 |
| [28: Final closure](prompts/28-final-closure.md) | Requirement-by-requirement completion audit | R01-R55 |

## Scope and external gates

All three crates belong to one Cargo workspace, as PRD section 5.1 specifies. The workspace is connected to ronnyabuto/daraja-engine. No Cargo manifests, implementation stubs or provider accounts have been created; prompt 00 handles the actual build foundation and preserves this repository.

The PRD labels itself verified, but that label is not technical evidence. Prompt 01 verifies upstream/CDK contracts and Safaricom behavior. In particular, B2B duplicate probing, asynchronous query trust, available receipt/amount fields, transaction lookup identifiers, custom payment units and legal assertions need actual sources and/or live evidence.

The stricter interpretation of mandatory query confirmation applies to all deposits. A pre-validation callback does not silently replace independent verification. A secret/correlated asynchronous response still needs an explicit threat model. Underpayment top-ups also need a remaining-amount validation policy; otherwise the PRD's initial exact-amount check would reject its own recovery flow. These ambiguities are recorded in [SPEC-NOTES.md](SPEC-NOTES.md), not silently resolved by inventing an API.

The three-week roadmap is the PRD's estimate, not a promised delivery time. Required local functionality remains required. Optional policy choices (partial overpayment settlement, grace windows and linked-account sweeps) must be explicit and safe; the optional PostgreSQL backend is included as a real implementation stage. Live-only credentials/evidence are never replaced by mock claims.

## What counts as done

A checked box is insufficient. Each requirement must link to implementation and executed boundary evidence. No required code path may return fabricated success. Mock-provider tests establish local handling, real CDK tests establish proof-lock semantics, live provider evidence establishes external behavior within its environment, and operator/counsel evidence establishes deployment prerequisites. Publishing/deploying is separately authorized.

The source is preserved in [PDF](../spec/prd-v1.0.pdf) and [text](../spec/prd-v1.0.txt). PDF wording wins if extraction differs; protocol contradictions require a documented compatibility decision.


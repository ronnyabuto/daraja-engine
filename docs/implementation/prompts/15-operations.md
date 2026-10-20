# Prompt 15: Operator reconciliation and auditable recovery

Requirement IDs: R28 R30 R36 R40
Prerequisite: stage 14 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/15-operations.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Implement a bounded operator service/CLI for listing pending/exception funds, viewing redacted evidence, retrying safe verification, requesting eligible refunds, re-quoting held FX deposits and resolving manual payouts. Use authenticated/authorized access with durable actor/reason/evidence audit records; ordinary application clients must not receive these powers.

Manual payout resolution must require transaction/portal evidence, amount/destination matching and compare-and-set transition protection. Marking unresolved money as Failed is forbidden. A resolution to Paid preserves the actual receipt; a definitive Failed resolution safely releases only the corresponding reservation and informs CDK. Handle duplicate commands and late contradictory provider evidence explicitly.

Connect refund execution to the same outbound saga and restrict destination to verified ownership/approved operator evidence. Complete exception allocation atomically so a refund cannot race mint settlement or a top-up. Expose low-float/manual escalation alerts and operational queue visibility, with durable delivery/retry semantics.

Acceptance: tests cover unauthorized operations, repeated/concurrent resolution, stale expected-state commands, paid/refunded fund reuse, ambiguous refunds, audit completeness, late success and operator restart. Provide a synthetic local drill demonstrating manual escalation remains Pending until valid resolution; include exact commands and expected outcomes in an operator runbook.


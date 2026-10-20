# Prompt 13: Ambiguity resolution and safe manual escalation

Requirement IDs: R35 R36
Prerequisite: stage 12 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/13-outbound-reconcile.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Implement the two-stage outbound reconciler with durable leases, configurable polling budget and MAX_PENDING_TTL. Auto-probing is separately gated for B2C and B2B by protocol/sandbox evidence for the deployed environment; default disabled without that evidence. Resubmit only the same durable operation with the original OriginatorConversationID and required payload equality. Duplicate 500.002.1001 indicates prior registration/in-flight status, never Paid.

Implement TransactionStatus reconciliation using only identifiers actually supported by the verified API. If an unacknowledged request has no queryable transaction ID and safe probing is unavailable, retain Pending and escalate. Result/query callbacks require the same documented verification/correlation protections as other settlement evidence.

On exhausted automated polling or the configured hard TTL, transition to MANUAL_RECONCILIATION_REQUIRED, emit a durable operator alert and keep the CDK-visible status Pending. Elapsed time, missing portal evidence, worker errors, shutdown or a query timeout can never authorize Failed. Paid requires verified settlement with an actual receipt; Failed requires documented definitive non-execution evidence or an audited authorized resolution.

Acceptance: fault/restart tests cover safely enabled probe, disabled B2B probe, duplicate rejection, lost callback, unqueryable ID, late success after manual escalation, concurrent reconcilers, contradictory callbacks and hard TTL. Prove no ambiguous path releases proof locks through status mapping.


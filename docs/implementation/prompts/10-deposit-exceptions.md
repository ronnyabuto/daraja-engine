# Prompt 10: Missing-money recovery, top-ups and refunds

Requirement IDs: R27 R28 R29 R30
Prerequisite: stage 09 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/10-deposit-exceptions.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Implement UNDERPAID_HELD, OVERPAID_HELD and ORPHAN_UNCLAIMED handling using verified deposit value and the persisted ledger. Default overpayments to a held exception, documenting the PRD's alternate partial-settlement/refundable-credit policy as optional rather than silently enabling it. Top-ups must accumulate distinct verified receipts toward the expected amount exactly once. Integrate C2B validation for the authorized remaining amount.

For late deposits, implement a configurable KES-only grace window. Expired sat/FX deposits remain held for an explicit re-quote or refund. Maintain auditable ownership/correlation for unknown references; do not allow a user to claim an orphan or choose a refund destination merely by supplying a receipt number.

Create durable refund requests tied to available exception balances and tariff deductions, routed through the same safe outbound saga once it exists. Until then keep execution gated, never a placeholder successful refund. Handle refund fee exceeding available funds, simultaneous top-up/refund and already-settled quotes without negative/double allocations.

Add configured velocity/settlement holds for high-value deposits from unverified MSISDNs. Document that organization-approved/Maker-Checker reversal configuration is an operator prerequisite outside software control.

Acceptance: tests exercise underpay + top-up, duplicate top-up, concurrent refund/top-up, overpay, unknown/late reference, KES grace, expired sat, hold release and simulated reversal alerts. No exception path emits mint settlement before verification and policy release.


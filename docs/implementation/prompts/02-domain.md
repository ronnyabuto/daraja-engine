# Prompt 02: State models and accounting invariants

Requirement IDs: R08 R09 R10 R11
Prerequisite: stage 01 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/02-domain.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Using the pinned protocol findings, implement the domain types and legal transitions in daraja-core. Write docs/state-machines.md with transition tables and the evidence required for each transition. Separate inbound ISSUED/STK_SENT/verification/hold/PAID states, outbound DISPATCHING/UNKNOWN_IN_FLIGHT/PENDING/PAID/definitively FAILED/MANUAL_RECONCILIATION_REQUIRED states, and deposit exception states.

Use checked integer/decimal amounts with explicit KES/sat units and no floating point. Distinguish quote expiry, provider attempt timing, FX expiry, settlement holds, tariff versions, receipt observations and verified value. Define stable request idempotency keys and canonical payload hashes; reject reuse with different payloads. Record both caller and provider correlation identifiers. Model multiple STK attempts under one quote and multiple deposits without double-credit.

Invariants: ambiguous payouts and manual escalation remain Pending to CDK; no timer alone releases proofs; settlement and durable event insertion are atomic; one actual receipt funds at most one accounting allocation; untrusted observations cannot irrevocably consume a receipt's confirmed-ledger slot; fees/rounding cannot create value; a settled quote cannot settle again via the other rail.

Acceptance: focused transition/property tests cover valid transitions, forbidden terminal regressions, overflow and currency mismatch, duplicate receipts, same-key/different-payload rejection, concurrent-rail scenarios, and the rule that manual reconciliation remains Pending. Use the established owner modules rather than building speculative framework abstractions.


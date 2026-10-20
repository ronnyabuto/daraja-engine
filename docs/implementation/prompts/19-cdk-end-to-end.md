# Prompt 19: Pinned cdk-mintd proof-lock integration

Requirement IDs: R04 R05 R44 R45
Prerequisite: stage 18 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/19-cdk-end-to-end.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Run the actual pinned cdk-mintd with cdk-daraja-processor and a controllable local Daraja simulator using disposable database/state. Connect a client through the real custom mpesa NUT-04/NUT-05 flow. Prove quote creation, verified deposit, blinded minting, token transfer/redeem and melt to phone/Till/PayBill with expected fee and M-Pesa receipt behavior.

Concentrate on proof state: inject HTTP 503, request timeout after the simulator accepts a transfer, dropped payout callback, gRPC disconnect after dispatch, processor restart and manual TTL escalation. Inspect cdk-mintd's actual persisted proof states through supported interfaces/test harness; a processor enum unit test is insufficient evidence. Proofs remain unavailable for a second spend while Pending, then reach the pinned CDK's correct final state only on verified settlement or definitive failure.

Check concurrent repeated MakePayment, event replay and CDK restart. If upstream cannot support the requested custom method, unit or pending semantics, record a reproducible blocker and implement the smallest justified local adapter correction; do not fork/patch upstream silently or claim the scenario passed.

Acceptance: retain synthetic fixtures, exact pinned versions, commands and redacted outcomes in evidence/19-cdk-end-to-end. All unsupported/live-only scenarios are labeled separately. Mock Daraja is appropriate here; the mint and processor boundary must be real.


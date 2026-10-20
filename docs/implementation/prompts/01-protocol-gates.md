# Prompt 01: Protocol research and executable verification gates

Requirement IDs: R03 R04 R05 R06 R07
Prerequisite: stage 00 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/01-protocol-gates.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Read PRD sections 2, 4, 5.2, 5.3 and Phase 0. Use current primary Safaricom and pinned CDK/NUT sources to verify endpoint versions, wire field spellings, identifiers accepted by TransactionStatus, query semantics, callback payload fields, C2B validation enablement/codes, and custom mpesa method/unit support. Compile a minimal pinned-CDK contract harness and test how a processor Pending response, gRPC error, disconnect, and restart affect mint proof locks. Trace source paths where a real harness cannot yet run.

Create docs/protocol-contract.md and docs/verification-gates.md with source URLs/revisions, exact observations, contradictions, and gated features. Build an opt-in sandbox probe harness with explicit credentials, redaction and non-production endpoints. B2C v3 AND B2B v1 duplicate rejection 500.002.1001 need independent evidence before automatic resubmission. A rejection is not proof of payment completion. Test payload equality as well as identifier reuse. Never run money-moving probes without explicit authorization.

Investigate whether asynchronous status-query callbacks supply independently trustworthy settlement evidence; their mere correlation with a query does not create a signature. Document available protections and unresolved trust assumptions. Likewise, query success may not attest every callback receipt/amount field. Never assume TransactionStatus can query by OriginatorConversationID without evidence.

Document legal verification tasks from section 2 as operator/counsel gates; distinguish enacted law from drafts and unverified PRD assertions, using current official legal sources for any asserted facts. Mock evidence cannot pass external gates.

Acceptance: executable local harness/probes, evidence classifications and fail-closed capability defaults. Missing credentials block live verification, not independent local stages.


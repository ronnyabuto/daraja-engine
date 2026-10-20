# Prompt 03: SQLite migrations and transactional repository

Requirement IDs: R09 R10 R11
Prerequisite: stage 02 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/03-sqlite.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Implement sqlx SQLite WAL persistence with ordered migrations for inbound_quotes, outbound_payouts, mpesa_receipts_ledger and unreconciled_deposits, plus the minimum attempt, durable-job/event, balance reservation and audit structures required by the state contract. Explain additions in the schema document. Persist scheduling and correlation fields so recovery does not depend on process memory.

Enforce unique account references, receipt identities, request idempotency keys and conversation correlation constraints at the database boundary. Persist request payload hashes and immutable outbound dispatch data before I/O. Implement atomic compare-and-set transitions, receipt verification/allocation, quote settlement plus event emission, and job claiming with bounded leases. Network I/O must not occur inside long database transactions. Configure busy timeouts, foreign keys and durability settings deliberately.

Receipt replay handling must distinguish a durable unverified observation, a resumable verification attempt, and a confirmed allocation. A forged first callback or crash must not permanently suppress a later legitimate settlement. Preserve observations and exception funds for reconciliation.

Acceptance: tests on a real file-backed SQLite database prove migration from empty state, reopen/recovery, competing writers, duplicate delivery, transaction rollback, job lease recovery and atomic settlement/event behavior. Include a crash boundary after state commit and before dispatch for later saga tests. Avoid in-memory-only tests for concurrency/durability claims.


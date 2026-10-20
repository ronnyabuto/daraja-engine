# Prompt 25: Security, race and crash recovery closure

Requirement IDs: R52
Prerequisite: stage 24 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/25-fault-audit.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Review the implemented financial paths against the original PRD and state invariants, then fix concrete failures within their owner modules. Use the installed system-reasoning skill for unclear cross-boundary failures. Work sequentially without delegating unless separately authorized. Preserve existing useful tests.

Run a deterministic fault matrix using a real file database, running daemon/processor and controlled provider responses: spoofed/wrong-secret callbacks, valid secret with wrong fields, forged receipt poisoning, replay, callback-before-ack, both inbound rails paid, under/over/late/orphan funds, receipt reuse across quotes, duplicate payout calls, HTTP 500/503, timeouts before/after provider acceptance, query throttling, query callback spoofing, worker crash/lease takeover, stream reconnect and operator resolution races.

Verify authorization boundaries, bind defaults, log/URL-secret redaction, customer phone privacy for B2B, rate/body limits, dependency advisories and credential file handling. Confirm immutable money fields, auditability, reservation safety and proof locks via real CDK evidence where affected.

Acceptance: retain an executable fault harness and a compact report with scenario, command, observed persistent state and outcome. Fix meaningful uncovered regressions with permanent tests. Do not mark an unknown provider guarantee resolved by a mock. Once affected checks pass, stop; do not expand into speculative redesign or an unrelated whole-repository cleanup.


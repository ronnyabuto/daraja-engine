# Prompt 14: Balance telemetry, reservations and optional sweeps

Requirement IDs: R37 R38 R39
Prerequisite: stage 13 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/14-liquidity.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Implement periodic AccountBalance requests, asynchronous correlation/result processing and persisted balance snapshots per collection/disbursement account. Expose daraja_b2c_utility_balance_kes and daraja_c2b_collection_balance_kes plus balance age, pending liability and alert metrics. Treat unavailable/stale observations conservatively; never assume C2B funds are immediately spendable by B2C.

Before accepting a melt quote, enforce available disbursement balance >= MIN_FLOAT_THRESHOLD + principal + tariff + active reservations. Use transactional reservation or equivalent concurrency-safe accounting; define expiry/release and recheck before dispatch without stranding ambiguous payouts. Account for pending transfers so old balance snapshots do not permit overcommit. Prevent stale out-of-order balance replies from restoring already-reserved/spent float.

Add optional BusinessTransferFromMMFToUtility orchestration only for explicitly linked shortcodes and verified capabilities. Reuse durable outbound safety and limits to avoid repeated sweep transfers. Unlinked or disabled accounts receive low-float alerts and operator bank/PesaLink instructions. No live sweeps in tests.

Acceptance: concurrent quote reservation, fee coverage, stale/unknown balance, quote expiry, restart, pending payout, out-of-order balance callback and low-float recovery tests. Optional sweep failure/timeout follows the same ambiguity rules. Metrics labels must avoid phone numbers, secrets and unbounded payment identifiers.


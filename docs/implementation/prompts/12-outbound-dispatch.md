# Prompt 12: Idempotent payouts and durable dispatch saga

Requirement IDs: R33 R34
Prerequisite: stage 11 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/12-outbound-dispatch.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Implement outward payment orchestration in daraja-core using immutable quotes and the state/repository contracts. Require stable operation idempotency plus payload equality; derive a deterministic OriginatorConversationID from the durable operation identity. Commit DISPATCHING, destination, amount/fee, original wire identity and correlation metadata before network I/O. Concurrent MakePayment calls and worker retries must claim one operation.

Dispatch once through the B2C/B2B transport. Persist acknowledgment and caller/provider IDs atomically enough for callback-race recovery; stage early callbacks durably for correlation. HTTP 500/503, disconnect, uncertain timeout, malformed acknowledgment and QueueTimeOutURL must lead to UNKNOWN_IN_FLIGHT/Pending. Distinguish documented definitive non-execution rejections from ambiguity; do not classify by HTTP status alone.

Recover records left DISPATCHING after a crash as potentially sent. Never generate a fresh OriginatorConversationID to escape uncertainty. Preserve the original business request for any later approved probe, including stable credentials/ciphertext fields where required by the verified contract, while treating expiring authentication separately.

Acceptance: mock-provider plus file-database tests cover same-key/different-payload, concurrent callers, crash before send, after send/before acknowledgment persistence, early callback, duplicate response, provider 200 with error, 500/503 and QueueTimeOutURL. Assert no path can issue two independent transfers for one operation or expose a false Failed status.


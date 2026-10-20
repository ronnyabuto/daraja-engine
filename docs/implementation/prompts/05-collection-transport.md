# Prompt 05: Daraja collection and query clients

Requirement IDs: R13 R14 R15
Prerequisite: stage 04 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/05-collection-transport.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Implement typed HTTP clients for STK processrequest, stkpushquery, C2B RegisterURL and TransactionStatus using the protocol findings. Route credentials by operation explicitly and preserve original provider error codes, HTTP outcomes, timeout phases and correlation IDs without logging secrets. Normalize provider strings/numbers defensively without accepting invalid financial amounts.

Use an injectable HTTP transport/provider base URL and clock. Enforce connect/request deadlines and classify spike arrest 500.001.1001 with bounded exponential backoff and jitter. Read-only query retry policy must remain separate from payment submission policy. Implement asynchronous TransactionStatus request/correlation persistence hooks and result/timeout DTOs without pretending the synchronous acknowledgment contains a final result.

C2B registration must include validation/confirmation URLs and the documented response policy; register only through an explicit operator operation, never as an uncontrolled startup side effect. Callback URL secrets must be redacted from outgoing request logs as well as ingress logs.

Acceptance: realistic mock-server tests assert exact paths, headers, documented field spelling, credential actor selection and serialization for all four endpoints. Cover HTTP errors, invalid JSON, non-success provider codes within HTTP 200, timeouts, query throttling and asynchronous acknowledgments. No test should require live Safaricom access.


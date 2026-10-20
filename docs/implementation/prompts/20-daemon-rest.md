# Prompt 20: Sidecar REST/OpenAPI and authenticated service lifecycle

Requirement IDs: R46
Prerequisite: stage 19 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/20-daemon-rest.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Expose existing core capabilities through daraja-daemon REST with a versioned OpenAPI contract. Include create/read inbound quote, STK resend, dual-rail instructions, create/read payout quote, idempotent execute/read payout, balance health and incoming settlement consumption. Operator exceptions/refunds/resolution use separate authorized routes or remain in the operator CLI. Specify idempotency headers, amounts/units, errors, TTLs, Pending/manual semantics and receipt fields explicitly.

Wire migrations, workers, callback ingress and metrics into one lifecycle with graceful shutdown. Configure loopback binding by default. Separate trusted control/API authentication from public Safaricom callbacks; network exposure requires explicit authentication/TLS deployment configuration. CORS and proxy trust are explicit. Provide health/liveness and readiness semantics that distinguish process health from provider/balance readiness without leaking secrets.

Generate/document OpenAPI from actual routes/types and check for drift. Prevent clients from setting provider completion state or bypassing fee/liquidity policies. Handle response loss and client retries through durable operation idempotency.

Acceptance: black-box HTTP tests against a running daemon with SQLite and mock provider: dual-rail deposit, cooldown, settlement, payout Pending then Paid, duplicate request, auth denial, malformed amount, stale balance and process restart. Fetch and validate the served OpenAPI document. No new business state machine belongs in route handlers.


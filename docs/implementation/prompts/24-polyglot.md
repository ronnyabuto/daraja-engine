# Prompt 24: Polyglot examples and integration AGENTS.md

Requirement IDs: R50 R51
Prerequisite: stage 23 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/24-polyglot.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Deliver runnable consumer examples for TypeScript/Node.js, Python, Go, PHP, C# and Rust, matching the executive integration boundary. Prefer minimal REST examples plus generated gRPC usage/build guidance where appropriate; do not invent six full SDKs. Each example demonstrates durable application idempotency, quote/deposit/status or stream flow, payout quote/execute/Pending recovery and actual receipt display. All access goes through the daemon, never raw database or provider credentials.

Complete root AGENTS.md's consumer integration section with concise architecture, prerequisites, exact REST/proto locations, auth, integer/unit conventions, example commands and provider-independent state semantics. Keep workspace contributor instructions clearly scoped and preserved. Provide a standalone docs/integration/AGENTS.md that consumers can copy without inheriting repository implementation tasks.

Explain callback ownership, retries with the same identity, HTTP timeout recovery by status, why Pending cannot be retried as a fresh payout, webhook/query trust gates and client-side limitations for custom mpesa Cashu wallets. Document C2B registration and the necessity of configured validation.

Acceptance: run/compile or syntax-check each example with its stated toolchain and run at least one collection and ambiguous payout recovery scenario against the actual local daemon per available runtime. Mark unavailable language runtimes as verification gaps; no claim of six-language execution from copied snippets. Confirm examples/OpenAPI/proto agree.


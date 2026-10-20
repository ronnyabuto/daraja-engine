# Prompt 22: Reference Cashu/mpesa CLI wallet

Requirement IDs: R48
Prerequisite: stage 21 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/22-reference-cli.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Build the minimal reference CLI needed for the pinned CDK custom mpesa method, using supported mint/wallet client APIs rather than a bolt11 assumption. Keep it under examples/reference-cli unless existing owners require otherwise. Support mint selection, KES/sat capability detection, quote/deposit, optional STK phone, immediate PayBill/reference/amount fallback, polling and server-authoritative 35-second resend cooldown.

Support actual ecash receive/send and melts to PHONE/TILL/PAYBILL as applicable to the PRD's off-rail bearer transfer claim. Show total principal/fees/expiry before execution, Pending/manual states without a misleading failure, and the actual M-Pesa TransactionID as cashier proof. Persist sensitive wallet state appropriately for a reference client and prevent token/proof/seed logging.

Use documented metadata lookup if the pinned CDK RPC cannot carry checkout details. Clearly separate reference-only deployment limitations from implemented workflows. No provider keys in the client.

Acceptance: scripted CLI scenarios with the real pinned local mint/processor and mock Daraja cover STK + fallback, no-phone deposit from another payer, token transfer, fee quote, all three exits, cooldown, expired quote, lost connection and Pending recovery. Capture exact commands/outputs with only synthetic secrets and tokens. Do not claim compatibility with mainstream wallets that hardcode bolt11.


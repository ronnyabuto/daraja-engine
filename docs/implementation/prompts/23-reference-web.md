# Prompt 23: Dual-rail reference checkout web client

Requirement IDs: R49
Prerequisite: stage 22 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/23-reference-web.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Build the lightweight reference web UI sharing the reference client's supported mpesa workflow and existing API contracts. Use a small maintainable stack; this is a payment integration example, not a new SaaS product. Display STK progress and immediate one-tap copy buttons for PayBill, account_ref and exact amount, quote expiry and a server-authoritative countdown before resend. Manual payment from a secondary phone must remain possible.

Provide wallet/mint flow needed to receive/mint and melt ecash through the pinned custom method, show phone/Till/PayBill target entry with validation, principal + fee + total confirmation, Pending/manual states and the final receipt with copy/display for a cashier. Keep client secrets/tokens out of logs and third-party analytics. Avoid exposing operator APIs or Safaricom credentials.

Handle refresh/reconnect and stale/out-of-order responses without enabling a second payment. Use accessible labels, keyboard controls and responsive layout. Explain any browser-wallet custody boundary and retain only the minimum sensitive client data.

Acceptance: run a real browser against the local stack and mock provider. Retain reproducible screenshots or short recordings of dual-rail checkout, disabled/enabled resend, phone/Till/PayBill quotes, Pending recovery and final receipt. Exercise mobile layout, copy actions, expired quote and refresh; do not count a build alone as UI verification.


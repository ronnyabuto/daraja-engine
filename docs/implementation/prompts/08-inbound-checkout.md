# Prompt 08: Durable inbound quotes and dual-rail checkout

Requirement IDs: R22 R23
Prerequisite: stage 07 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/08-inbound-checkout.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Implement inbound quote creation with collision-safe eight-character alphanumeric account_ref, shortcode, exact expected M-Pesa amount, unit, TTL and durable idempotency. If an MSISDN is supplied, create a persisted STK attempt, dispatch through the collection client and store CheckoutRequestID. Always make C2B PayBill + reference + exact amount instructions available immediately, including when STK dispatch is unavailable. Do not turn uncertain STK submission into a new automatic duplicate prompt.

Return the checkout data model needed by both adapters/reference clients: quote identifier, payment state, attempt identifier, STK status, fallback instructions, absolute retry_after and quote expiry. Enforce the 35-second retry cooldown server-side with the same persisted attempt timing used by polling. A resend must reconcile prior attempts and remain under one quote; late callbacks for older attempts cannot create a second settlement.

KES behavior is fully usable now; wire sat quotes only through an explicit FX capability interface and keep sat unsupported until stage 11 is complete. No fabricated rates.

Acceptance: service-level tests cover concurrent idempotent creation, reference collisions, no-phone/manual-only checkout, successful/failed/ambiguous STK dispatch, server-side early resend rejection, eligible resend and restart-preserved cooldown. Demonstrate immediate manual fallback and a secondary payer phone without requiring the original phone to match.


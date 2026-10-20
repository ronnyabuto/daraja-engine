# Prompt 18: CDK quote/pay/check outgoing and proof safety

Requirement IDs: R43 R44
Prerequisite: stage 17 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/18-cdk-outgoing.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Implement GetPaymentQuote, MakePayment and CheckOutgoingPayment against the pinned service contract. Quotes must bind immutable destination, unit, amount, tariff, FX TTL and float reservation. Repeated MakePayment for the same identity resumes the same core saga; it never starts a new payout or reparses a modified destination.

Map unknown, DISPATCHING, in-flight and MANUAL_RECONCILIATION_REQUIRED states to the pinned CDK Pending representation. Determine from the actual cdk-mintd implementation how gRPC errors/cancellation are interpreted; avoid a transport/error mapping that releases proofs after a possible dispatch. Disable unsupported unit/method combinations accurately.

Return the real M-Pesa TransactionID in the documented proof/preimage field only after verified payment. Handle the pinned protocol's exact fee and amount semantics; do not pass receipt-shaped fake hashes or expose the customer's MSISDN for B2B. Confirm definitively failed mapping only for safe evidence classes.

Acceptance: tonic boundary tests cover phone/Till/PayBill, invalid/expired quote, stale float, concurrent repeated MakePayment, successful receipt, proven rejection, HTTP 500/503, timeout, QueueTimeOutURL, restart and manual escalation. Assert Pending under every ambiguous case and field-correct receipt/fee reporting. Preserve proto compatibility and explain any actual upstream adapter limitation.


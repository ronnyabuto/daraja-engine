# Prompt 06: B2C, B2B and balance wire adapters

Requirement IDs: R16 R17 R18 R19
Prerequisite: stage 05 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/06-payout-transport.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Implement typed adapters for B2C v3 BusinessPayment, B2B v1 BusinessBuyGoods and BusinessPayBill, AccountBalance and the opt-in BusinessTransferFromMMFToUtility command. Use the exact verified field names, identifier types, environment certificate and app credentials. Persist/carry caller OriginatorConversationID and all provider-returned identifiers; do not assume the proxy preserves the caller ID.

Validate destinations for normalized Kenyan phone forms including 07/01 and 2547/2541 where supported, TILL:<number>, and PAYBILL:<shortcode>:<account_ref>. Reject malformed/ambiguous destinations and invalid provider length/range constraints. B2B requests use corporate remitter information and never require the customer's MSISDN. Preserve receipt fields for final cashier proof.

No automatic retries of money-moving requests in this transport layer. Return sufficiently rich outcomes for the saga to distinguish acknowledgment, proven rejection, duplicate, and uncertainty. AccountBalance and TransactionStatus acknowledgments schedule durable asynchronous correlation; timeout callbacks are not final failure evidence.

Acceptance: mock HTTP contract tests for every command, destination and identifier combination, malformed/error payloads, duplicate code, timeout and HTTP 500/503. Inspect serialized B2B requests and logs to prove the customer's phone number is absent. Sweep capability defaults off and requires linked-account configuration and verified command support.


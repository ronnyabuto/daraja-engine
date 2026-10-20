# Prompt 27: Sandbox validation and operator readiness gates

Requirement IDs: R06 R07 R55
Prerequisite: stage 26 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/27-live-verification.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Revisit every external gate established in stage 01 using the now-complete stack. Prepare a redacted, reproducible opt-in sandbox/staging acceptance plan for STK, C2B RegisterURL/validation/confirmation, query results, B2C, B2B Till/PayBill, balance and custom mpesa client compatibility. Execute only the live calls authorized for the configured non-production account; ask a single focused question if necessary for missing access/authorization and continue independent validation.

Verify B2C and B2B OriginatorConversationID duplicate behavior independently, including identical-payload retries after an uncertain response and environment-specific response/error fields. Confirm provider settlement count/receipts and the API's actual transaction lookup capabilities. Capture query/callback trust limitations and provider/portal evidence. Sandbox evidence must not be represented as proof of production guarantees; require corresponding provider documentation/production-safe validation before enabling deployment-specific auto-retry.

Check operator tariff sources, credentials/permissions, linked-shortcode sweep eligibility, maker-checker reversal settings, float thresholds and live FX provider configuration. Ask operators/counsel to supply legal classification evidence; do not issue a legal clearance based on the PRD.

Acceptance: update each gate to PASS, BLOCKED or UNSUPPORTED with dated evidence, capability defaults and the exact next action. Missing live evidence cannot be relabeled PASS using mocks. The local implementation may be complete while production readiness remains blocked.


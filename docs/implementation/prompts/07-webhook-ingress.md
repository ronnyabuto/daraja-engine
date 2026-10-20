# Prompt 07: Secure callback ingress and C2B validation

Requirement IDs: R20 R21
Prerequisite: stage 06 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/07-webhook-ingress.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Implement axum ingress for /callbacks/stk/{secret}, /callbacks/c2b/validation/{secret}, /callbacks/c2b/confirmation/{secret}, /callbacks/b2c/{secret}, /callbacks/b2b/{secret}, and the verified result/timeout endpoints needed by status and balance queries. Centralize ingress policy with endpoint-specific DTO validation. Use high-entropy configured URL secrets with constant-time comparison, request body bounds and safe error responses. Optional CIDR filters are supplementary and must handle proxy trust explicitly; never present the PRD's community CIDRs as an authoritative allowlist.

Persist accepted observations/jobs before acknowledging confirmation/result callbacks. C2B synchronous validation must return quickly, check an active quote and amount, and emit the verified invalid-account/invalid-amount codes C2B00012/C2B00013 if supported. Document enablement and timeout/default-response behavior. Permit only the approved remaining amount for a held top-up under the exception policy, avoiding a dead-end caused by the original exact-amount check.

Bind STK by stored CheckoutRequestID and C2B by account_ref/collection shortcode using fields actually present in each payload. Do not require absent STK BusinessShortCode or fabricate it. Invalid/spoofed callbacks cannot settle funds or corrupt verified receipt allocations.

Acceptance: route-level tests cover invalid secrets, oversized/malformed payloads, quote mismatch, wrong shortcode, duplicate callbacks, proxy spoofing, C2B typo/amount rejection and durable acknowledgment. URL secrets and MSISDNs must not appear in access/error logs.


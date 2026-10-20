# Prompt 28: Requirement-by-requirement completion audit

Requirement IDs: R01-R55
Prerequisite: stage 27 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/28-final-closure.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Audit every requirement row in docs/implementation/requirements.md against the original PDF, implementation and executed evidence. Read only the relevant source/test ranges per row; do not rely on prior summaries or checkbox claims. Confirm all seven CDK RPCs, daemon REST/gRPC, two databases, FX/tariffs, liquidity, exceptions/refunds, both reference clients, six language examples and packaging have real implementations.

For missing local behavior, fix it and run the affected checks. Replace placeholder success, TODO/unimplemented execution paths and disabled features presented as delivered with actual behavior or explicit unsupported/blocker entries. Keep intentional external gates fail-closed. Review naming/comments/scaffolding for consistency without unrelated refactors.

Re-run the release-required check set after final edits, and demonstrate one collection-to-mint-to-token-transfer-to-melt flow plus ambiguous payout recovery on the pinned local stack. Verify source PDF provenance, public schema docs and examples match the code. Preserve redacted evidence and a concise current handoff.

Acceptance: create docs/completion-report.md with each requirement's implementation/test references, exact final commands/results, unresolved upstream/provider/legal gates and capability restrictions. Distinguish implementation complete, locally verified, live verified, production ready and published. Never claim complete v1.0 if required local behavior is absent, or production ready while required external gates are open. Report the smallest concrete next action for every blocker, and stop once the requested checks pass.


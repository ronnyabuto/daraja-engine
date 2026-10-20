# Prompt 26: Reproducible releases and operator deployment

Requirement IDs: R53 R54
Prerequisite: stage 25 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/26-packaging.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Prepare release artifacts for the library and the two binaries. Add reproducible CI for formatting, clippy, builds, meaningful tests, SQLite/PostgreSQL feature coverage, proto/OpenAPI drift and reference example checks. Run required checks on actual supported toolchains; do not hide failing test matrices behind blanket allow-failure.

Provide non-root container and service deployment examples, configuration reference, database migration/backup/restore/upgrade instructions, TLS/control-vs-callback routing, health/metrics and graceful shutdown. Produce musl/static artifacts if supported by dependencies and document actual target/linkage and measured stripped size; the PRD's ~15MB is a target to measure, not a fabricated guarantee.

Complete README, changelog, dual licenses, crate metadata, operator liquidity/reversal/credential guidance, exception/refund/manual resolution runbooks and explicit capability limitations. Prepare cargo package/publish dry-runs in dependency order, ensuring the processor/daemon resolve the intended core version without relying only on local path overrides. A fresh unpacked package must build.

Acceptance: clean checkout/build, container startup/health, config validation, package dry-runs and a real backup/restore drill that resumes unresolved payouts safely. Record release prerequisites and remaining external gates. Prepare a release; publishing crates/images, deploying publicly or moving production money requires explicit user authorization unless already granted. Do not manufacture published URLs.


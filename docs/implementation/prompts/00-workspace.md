# Prompt 00: Workspace and pinned build foundation

Requirement IDs: R01 R02 R03
Prerequisite: prepared folders and local specification only.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/00-workspace.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Create the minimum buildable Rust Edition 2024 Cargo workspace containing crates/daraja-core (library), crates/cdk-daraja-processor (library and binary), and crates/daraja-daemon (binary). Preserve the supplied planning files. Initialize one Git repository at the workspace root if none exists; apply the installed new-feature skill once, adapting honestly to a new repository without origin/main. Record the assigned branch/worktree so subsequent stages preserve it.

Choose supported Rust, tokio, axum, tonic/prost and sqlx versions after checking official documentation for uncertain compatibility; pin the toolchain and commit an application lockfile. Inspect upstream cashubtc/cdk, select and record an exact tag and commit, obtain its real CdkPaymentProcessor proto with source provenance/license/checksum, and compile generated types from it. Do not invent the seven-RPC wire schema. If upstream differs from the PRD, record the exact mismatch and a proposed compatibility decision before building adapter behavior.

Add dual MIT/Apache-2.0 license files, configuration example with placeholders, secret/build ignores, formatting and proportionate CI checks. No network payment behavior yet.

Acceptance: all three crates build, proto generation is reproducible, a clean build has documented prerequisites, and no credentials are tracked. Record exact tool/dependency/proto versions in docs/protocol-contract.md. An unavailable upstream schema is a real adapter blocker, not permission to fabricate it.


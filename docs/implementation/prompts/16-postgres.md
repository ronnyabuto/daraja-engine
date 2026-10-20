# Prompt 16: Optional PostgreSQL parity

Requirement IDs: R41
Prerequisite: stage 15 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/16-postgres.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Add the PRD's optional PostgreSQL backend behind an explicit configuration/feature boundary while retaining SQLite as the zero-config default. Reuse the domain and repository owner contracts; avoid copying business state machines into database-specific modules. Implement equivalent migrations, uniqueness, transactional receipt allocation, job leasing, idempotency, quote reservations and durable settlement events using database-appropriate SQL and locking.

Verify portability of enum/state encoding, numeric precision, timestamps, conflict handling and migration ordering. Keep production SQL compile validation reproducible; document how CI obtains sqlx metadata or a test database. Do not claim PostgreSQL support based on successful SQLite tests.

Acceptance: run the repository contract suite against a real ephemeral PostgreSQL instance, including concurrent workers and serializable/transaction retry behavior, receipt races, same-key conflicts, balance reservation overcommit prevention, job lease recovery and restart. Compare user-visible state/outcomes across both backends. Capture actual database version and commands. If Docker/database access is unavailable, implement what can be checked locally and leave the PostgreSQL verification gate blocked rather than marking parity done.


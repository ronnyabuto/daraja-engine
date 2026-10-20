# Daraja workspace instructions

This is one Rust workspace. Read README.md and the relevant protocol, state and schema contracts before editing. Public progress is recorded in docs/roadmap.md. Internal sequential prompts and handoffs may be present in the ignored docs/implementation directory; when assigned a numbered stage, follow those local instructions.

## Contributor workflow

- Before changing existing code inspect behavior, callers, tests and constraints. Keep logic with its owner; extract only concrete duplication.
- Apply the installed new-feature skill once for new Git implementation, preserving the assigned branch/worktree thereafter. A fresh local repository may have no origin/main; record that honestly. Do not create nested repositories inside crates.
- Use current-docs for uncertain version-sensitive integrations and system-reasoning for architectural tradeoffs or unclear cross-component failures.
- Do not delegate unless the user or applicable skill explicitly requests it.
- Implement one numbered prompt at a time. Preserve meaningful tests and verify the affected boundary with representative failure/recovery outcomes.
- For numbered implementation work, maintain local docs/implementation/STATUS.md and requirements.md plus concise reproducible evidence. Keep those files and the source PRD local; do not force-add ignored planning material. Update public technical contracts when behavior changes. Context compaction is not completion.
- Remove redundant scaffolding/comments and follow local naming. Stop when requested behavior and relevant checks pass.

## Financial invariants

- A possibly dispatched payout remains Pending through transport errors, timeouts, worker crashes and manual escalation. A timer is never proof of failure.
- Commit durable payout identity before I/O; reuse immutable operation identity/payload for any evidence-approved probe.
- No callback alone authorizes mint settlement. Verify quote, amount, receipt allocation and independently available provider evidence.
- Unverified receipt observations cannot permanently block a legitimate receipt; receipt allocation and settlement/event writes are transactional.
- Money uses checked exact amounts/units. Tariffs, FX expiry and liquidity reservations cannot be bypassed through either API.
- Provider duplicate-rejection guarantees are environment/endpoint gates; mock tests do not establish them.
- Logs omit secrets, full callback URLs, customer MSISDNs and bearer wallet material.

## Consumer integration guide

This section is intentionally pending stage 24. Until the daemon contracts and runnable examples exist, this file does not claim an integration API is available. Stage 24 adds the actual guide and docs/integration/AGENTS.md while preserving contributor instructions above.


# Resume the active stage

Paste this into a new Codex session, with GPT-6.1-Sol and medium selected:

```text
Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine.
Read AGENTS.md and docs/implementation/STATUS.md, inspect git status and the
recorded branch/worktree, then read the active numbered prompt, its requirement
rows and its latest evidence. Verify the actual implementation before relying
on the handoff. Preserve user changes and existing pins.

Resume the first unfinished acceptance item of the active stage. Do not replay
successful money-moving operations, start a fresh payout identity, repeat the
new-feature setup, advance past a failed prerequisite, or claim a mock passes a
live gate. Continue independent local work while external gates remain disabled.
Run affected checks and update the handoff/evidence. Stop when this stage's
acceptance criteria pass, or checkpoint exact remaining work if still blocked.
```

When a stage is DONE, use the next numbered prompt instead. For missing credentials/provider evidence, record which feature is gated, what local work remains possible and the smallest external action needed. Never mark the whole product production-ready merely because local tests pass.


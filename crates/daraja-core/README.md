# daraja-core

Owns financial state, persistence, provider adapters and reconciliation within the parent Rust workspace.

Stage 02 implements exact KES/sat amounts, canonical request identity, inbound and outbound transitions, verified receipt allocation and the settlement/event domain contract. See [state machines](../../docs/state-machines.md) and [local evidence](../../docs/implementation/evidence/02-domain.md).

Persistence and live provider verification are not implemented here yet. Next is [prompt 03](../../docs/implementation/prompts/03-sqlite.md), which must enforce these contracts transactionally on SQLite. No external capability is enabled by synthetic domain tests.

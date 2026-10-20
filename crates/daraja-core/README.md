# daraja-core

Owns financial state, persistence, provider adapters and reconciliation within the parent Rust workspace.

Stage 02 implements exact KES/sat amounts, canonical request identity, inbound and outbound transitions, verified receipt allocation and the settlement/event domain contract. See [state machines](../../docs/state-machines.md) and [local evidence](../../docs/implementation/evidence/02-domain.md).

Stage 03 adds SQLx SQLite WAL migrations and a transactional repository with durable observations, allocations, jobs, events, reservations and audit records. See [schema and recovery contracts](../../docs/schema.md) and [stage 03 evidence](../../docs/implementation/evidence/03-sqlite.md). Live provider verification and workers remain later stages; every external capability stays disabled. Next is [prompt 04](../../docs/implementation/prompts/04-auth-crypto.md).

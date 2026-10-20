# daraja-core

Owns financial state, persistence, provider adapters and reconciliation within the
parent Rust workspace.

Implemented and locally tested:

- Exact KES/sat amounts, canonical request identity, guarded inbound/outbound
  transitions and receipt allocation: [financial contracts](../../docs/state-machines.md).
- SQLx SQLite WAL migrations, transactional settlement, durable jobs/events,
  reservations and audit records: [schema and recovery](../../docs/schema.md).
- Isolated OAuth actors, EAT STK passwords and X.509 RSA credentials:
  [authentication and cryptography](../../docs/auth-crypto.md).
- STK processrequest/query, explicit C2B registration and asynchronous
  TransactionStatus clients: [collection transport](../../docs/collection-transport.md).

Provider verification, callback ingress, payout transports and reconciliation
workers remain under development. Local tests do not establish live provider
permissions or settlement evidence. See the [roadmap](../../docs/roadmap.md) and
[verification gates](../../docs/verification-gates.md).

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

- Gated B2C/B2B payout and balance clients, immutable dispatch preparation and
  asynchronous observations: [payout transport](../../docs/payout-transport.md).

- Secure callback ingress and durable checkout: [ingress](../../docs/webhook-ingress.md)
  and [checkout](../../docs/inbound-checkout.md).
- Leased inbound reconciliation, transactional verified settlement and durable
  notification replay: [reconciliation](../../docs/inbound-reconcile.md).

Live independent provider verification remains gated. Local tests do not establish
live provider permissions or settlement evidence. Liquidity telemetry, quote
reservations and optional bounded sweeps are library APIs:
[liquidity contract](../../docs/liquidity.md). See the [roadmap](../../docs/roadmap.md) and
[verification gates](../../docs/verification-gates.md).

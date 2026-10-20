# Implementation roadmap

Stages 00–08 are implemented and locally verified. This is a development
workspace, not a runnable payment service or a production-ready mint integration.

| Stages | Scope | State |
|---|---|---|
| 00–01 | Pinned Rust workspace/CDK schema, protocol research, local gRPC tests and disabled capability gates | Locally verified |
| 02–03 | Exact amounts, financial state machines, transactional SQLite persistence and recovery primitives | Locally verified |
| 04 | Isolated OAuth actors, EAT STK passwords and RSA credentials | Locally verified |
| 05 | Collection/query transports, explicit C2B registration and asynchronous status hooks/DTOs | Locally verified |
| 06 | Gated payout/balance transports, immutable preparation and asynchronous correlation hooks | Locally verified |
| 07 | Secure callback ingress, durable inbox/jobs and gated synchronous C2B validation | Locally verified |
| 08 | Durable idempotent inbound quotes, immediate dual-rail fallback and reconciled resend cooldown | Locally verified |
| 09–10 | Independently verified inbound reconciliation and deposit exceptions | Planned |
| 11–15 | Tariffs/FX, durable payout dispatch/reconciliation, liquidity and operator recovery | Planned |
| 16 | Optional PostgreSQL parity | Planned |
| 17–19 | CDK service implementation and real mint/proof-lock integration | Planned |
| 20–24 | Daemon REST/gRPC, reference clients and consumer examples | Planned |
| 25–28 | Fault/security closure, packaging, live verification and final requirement audit | Planned |

The current workspace test suite has 95 top-level test entries, including SQLite
child-process helpers. Tests cover domain invariants, file-backed SQLite
transactions and crash recovery, OAuth coordination, synthetic cryptography,
collection/query/payout/balance HTTP contracts, callback admission/durability/C2B validation,
durable checkout creation/resend and dual-rail fallback, deadline phases, throttling
and TRACE redaction.
See [build and verification commands](../README.md#build-locally) to reproduce them.

Status persistence hooks establish call ordering and failure handling; durable
status workers and restart reconciliation remain planned. Provider callbacks and
successful queries never supply settlement authority by themselves. Both binaries
currently exit with an explicit diagnostic until their service implementation is
available.

All external [verification gates](verification-gates.md) remain disabled. Local
mock tests do not establish live provider permissions, duplicate guarantees,
callback authenticity, regulatory clearance or combined mint compatibility.

Contributor-facing technical contracts and reproducible tests are distributed
with the source. Internal prompts, session handoffs, machine-specific evidence and
original specification files are kept in ignored local directories. Synthetic
certificate/key fixtures are intentionally public test data; see their
[provenance](../crates/daraja-core/tests/fixtures/README.md).

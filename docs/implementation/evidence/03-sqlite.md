# Stage 03 — SQLite migrations and transactional repository

Date: 2026-10-20. Local acceptance: **DONE**. R09/R10 local persistence acceptance
is complete; R11's repository primitives are complete, while provider workers and
end-to-end saga recovery remain stages 09/12/13. No external gate changed.

## Checkout, prerequisite and inputs

- Assigned checkout `/home/officialnyabuto/Desktop/daraja-engine`, branch `main`.
  Initially clean. Preserved this recorded continuation; no branch switch/reset,
  worktree creation, delegation, commit, push or publication occurred.
- Applied new-feature once to verify the continuation. Initial sandbox fetch
  failed because `.git/FETCH_HEAD` is read-only; authorized escalated `git fetch
  origin` succeeded. Both HEAD and origin/main were
  `fe468cb9e8556cde1294b91f2db15cd800879a13`. Stage 03 verification is against
  **dirty/uncommitted changes** based on that revision, including new untracked
  migrations/repository/tests/docs; not a claim of an already committed revision.
- Read AGENTS.md, STATUS.md, requirements.md, stage 03 prompt, financial state
  contracts, protocol findings, capability register and existing domain callers/
  tests. Stage 02 local acceptance was DONE. LIVE, DUP-B2C, DUP-B2B, SETTLE,
  STATUS-ID, C2B-VALIDATE, CDK and legal gates remain disabled/separately blocked.
- `pdftotext -layout docs/spec/prd-v1.0.pdf /tmp/daraja-prd-stage03.txt` then
  `cmp docs/spec/prd-v1.0.txt /tmp/daraja-prd-stage03.txt`: success, identical.
  Reviewed §§4, 5.2/5.3 and implementation schema requirements against the PDF text.
- Applied system-reasoning to preserve domain ownership and separate receipt
  claims, verification work, confirmed custody and settlement. Chose indexed
  relational constraints plus private full domain snapshots instead of duplicating
  financial rules in SQL or loading the entire database into process memory.

## Implementation and version evidence

`crates/daraja-core/migrations/000{1,2}_*.sql`, `src/repository.rs`, private storage
codecs in the domain owners/`src/storage.rs`, `build.rs` and `docs/schema.md`:

- Ordered SQLx migrations with checksum enforcement; WAL/FULL, foreign keys,
  five-second busy timeout on every connection. Bounded acquisition/write locks.
- Database identity/correlation constraints, immutable dispatch/body/hash and
  request binding triggers; exact u64 financial snapshots and decimal reservation
  amounts; checked signed-range scheduling times.
- Short immediate transactions, version CAS, domain-validated financial changes,
  transactional receipt allocation/exception custody, quote/receipt settlement and
  unique durable event emission/replay. No provider/network I/O inside transactions.
- Independent durable observations and verification jobs; duplicate receipt claims
  never reserve confirmed identities. Persisted STK/payout correlation and timing;
  1–300-second leases with increasing fencing generations and resumable due times.
- Payout preparation atomically commits identity/hash/body, request binding,
  reservation, audit and reconciliation job before returning to future transport.
  Ambiguity/manual escalation retains Pending and the reservation. Evidence-bound
  terminal resolution and reservation release share a transaction.
- Snapshot decoding is private rather than a public Deserialize bypass around
  domain constructors. SQL statement logging and error payload details are omitted.

Used current-docs for consequential SQLx 0.8.6 uncertainty, retaining its existing
pin. Official [Pool::begin_with](https://docs.rs/sqlx/0.8.6/sqlx/struct.Pool.html#method.begin_with)
and [migrate!](https://docs.rs/sqlx/0.8.6/sqlx/macro.migrate.html) were read. The
SQLite options docs URL failed to render; verified `foreign_keys`, `journal_mode`,
`synchronous` and `busy_timeout` in the installed exact-version
`sqlx-sqlite-0.8.6/src/options/mod.rs`, and exercised them on all four real pooled
connections. Enabled `migrate`/`macros` on existing SQLx; added explicit serde
1.0.229/serde_json 1.0.154 and test-only tempfile 3.27.0 already present transitively.
Cargo.lock records SQLx macro/migration dependency additions; direct existing pins
were not upgraded. Cargo's first dependency download failed on read-only cache;
escalated `cargo test -p daraja-core --no-run` downloaded/compiled successfully.

## Reproducible checks and actual results

All database tests use temporary **file-backed** SQLite files, not in-memory
connections. Toolchain remains rustc/cargo 1.93.0.

| Command | Result |
|---|---|
| `cargo test -p daraja-core --test sqlite --locked` | Initial 12 test entries passed; expanded with process writers, ordered upgrades and exact-value/FX cases below. |
| `cargo test --workspace --locked` | Final PASS: 23 existing domain tests, 17 SQLite integration entries (15 acceptance scenarios plus 2 subprocess entry helpers), 1 pooled-connection configuration test and 3 existing protocol integration tests. Doc tests passed. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS after final implementation/test changes. |
| `cargo fmt --all -- --check` | PASS. |
| `python3 scripts/check-foundation.py` | PASS pinned contract/configuration/tracked-file checks. |
| `python3 scripts/test-protocol-gates.py` | PASS, 6 tests. |
| `git diff --check` | PASS. |

Acceptance evidence in `crates/daraja-core/tests/sqlite.rs`:

- `migrations_reopen_and_connection_settings`: empty database migrations, reopen,
  quote/audit survival and WAL; repository unit test checks foreign keys=1,
  synchronous=2 (FULL), busy_timeout=5000 and WAL on all four connections.
- `ordered_upgrade_preserves_rows_and_checksum_tampering_fails_closed`: real v1
  database with data upgrades to v2, preserving quote; modified recorded migration
  checksum makes open fail.
- `forged_first_callback_verification_crash_duplicate_and_settlement_replay`:
  spoof persists, original verification lease expires across close/reopen, a later
  legitimate observation of the same receipt remains admissible, old lease is
  fenced, trusted synthetic evidence settles exactly once, delivery/receipt replay
  cannot settle twice, observation history and event cursor survive reopening.
- `competing_connections_cannot_allocate_or_settle_twice` and
  `concurrent_duplicate_delivery_and_topups`: independent pools compete through
  SQLite; one settlement/receipt allocation wins, distinct extra funds remain in
  exception custody, distinct verified top-ups settle together.
- `database_failure_rolls_back_all_settlement_writes`: injected database trigger
  aborts event insertion after preceding quote/receipt writes; quote, ledger,
  event and audit roll back; removing the trigger allows the same receipt to settle.
- `invalid_binding_cas_and_overpayment_do_not_mint`: wrong shortcode cannot consume
  receipt, stale version cannot settle, overpayment and orphan funds remain held.
- `idempotent_payout_reopen_immutable_dispatch_and_pending_reservations` and
  `competing_request_keys_and_job_leases_are_atomic`: payload conflict rejection,
  original operation returned on retry, immutable bytes, stale CAS rollback,
  Pending through recovery/manual escalation, reservation retained until trusted
  terminal evidence and single job claim under competition.
- `account_attempt_and_provider_correlations_are_database_unique`: duplicate
  account, cross-quote checkout and cross-payout provider conversation rejected;
  failed acknowledgment rolls back; foreign-key and audit immutability checks.
- `expired_job_is_retried_and_stale_worker_is_fenced`: expiry equality, generation
  fencing, scheduled retry, completion and maximum lease enforcement.
- `separate_process_writers_bind_one_request_and_reservation`: two child processes
  with independent runtimes/pools compete on one file; exactly one payout,
  idempotency binding, reservation and reconciliation job survive.
- `process_exit_after_commit_before_dispatch_and_uncommitted_write_recover`: child
  process exits 71 without Rust destructors/pool shutdown after committed payout
  preparation, with **no transport invocation**. A second case exits while a raw
  write transaction is still alive. Parent reopens file and verifies DISPATCHING /
  CDK Pending, exact dispatch, request binding, reservation and reconciliation job;
  the uncommitted state update is absent. This is the later saga's explicit crash
  boundary, not proof of provider execution or an actual power-loss test.
- `unsigned_money_roundtrips_and_overflow_rolls_back`: full u64 monetary range
  survives reopening, checked sum overflow does not consume a receipt, final
  exact top-up settles with correct credit.
- `persisted_fx_and_hold_deadlines_still_gate_settlement_after_reopen`: rational
  FX/sat snapshot survives reopening, hold and FX expiry reject settlement without
  an event, valid time credits the exact sat amount.

During development the private codec generator initially truncated an array type;
`cargo fmt` detected the syntax error before test execution, and it was corrected.
All final checks above passed; no tests or requirements were removed or weakened.

## Limits and next action

Synthetic statement attestations exercise a trusted repository boundary, not
provider authentication. No live credentials, real customer data or money movement
were used. No live capability is enabled; external BLOCKED gates remain unchanged.
Power-loss hardware guarantees, backups/restore, provider workers, operator
workflows, liquidity admission and combined CDK/provider saga faults remain their
scheduled stages. Payout jobs authorize reconciliation, never automatic resend.

Next: execute `docs/implementation/prompts/04-auth-crypto.md` in this same checkout,
preserving branch and these changes. Do not repeat branch setup or advance to
stage 05 in the same numbered task.

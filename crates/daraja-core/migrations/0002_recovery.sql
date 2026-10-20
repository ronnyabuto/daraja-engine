CREATE TABLE durable_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('VERIFY_RECEIPT','RECONCILE_PAYOUT','POLL_STK')),
    subject TEXT NOT NULL,
    due_at INTEGER NOT NULL CHECK(due_at>=0),
    lease_until INTEGER,
    generation INTEGER NOT NULL DEFAULT 0 CHECK(generation>=0),
    completed_at INTEGER,
    UNIQUE(kind,subject)
) STRICT;
CREATE INDEX jobs_due ON durable_jobs(completed_at,due_at,lease_until);
CREATE TABLE balance_reservations (
    payout_id TEXT PRIMARY KEY NOT NULL REFERENCES outbound_payouts(id),
    amount TEXT NOT NULL CHECK(length(amount)>0 AND amount NOT GLOB '*[^0-9]*'),
    unit TEXT NOT NULL CHECK(unit='KesMinor'),
    released_at INTEGER,
    evidence TEXT
) STRICT;
CREATE TABLE audit_records (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    subject TEXT NOT NULL,
    action TEXT NOT NULL,
    created_at INTEGER NOT NULL CHECK(created_at>=0)
) STRICT;
CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit_records
BEGIN SELECT RAISE(ABORT, 'append-only audit'); END;
CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit_records
BEGIN SELECT RAISE(ABORT, 'append-only audit'); END;

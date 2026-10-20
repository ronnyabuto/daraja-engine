CREATE TABLE deposit_policy (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    data TEXT NOT NULL CHECK(json_valid(data))
) STRICT;
CREATE TABLE deposit_risk (
    receipt TEXT PRIMARY KEY REFERENCES mpesa_receipts_ledger(receipt) DEFERRABLE INITIALLY DEFERRED,
    subject TEXT NOT NULL,
    verified_at INTEGER NOT NULL CHECK(verified_at>=0)
) STRICT;
CREATE INDEX risk_velocity ON deposit_risk(subject,verified_at);
CREATE TABLE deposit_blocks (
    quote_id TEXT PRIMARY KEY REFERENCES inbound_quotes(id),
    reason TEXT NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;
CREATE TABLE deposit_alerts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    receipt TEXT NOT NULL REFERENCES mpesa_receipts_ledger(receipt) DEFERRABLE INITIALLY DEFERRED,
    kind TEXT NOT NULL,
    evidence TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(receipt,kind,evidence)
) STRICT;
CREATE TABLE refund_requests (
    id TEXT PRIMARY KEY,
    data TEXT NOT NULL CHECK(json_valid(data)),
    state TEXT NOT NULL CHECK(state='EXECUTION_GATED'),
    created_at INTEGER NOT NULL
) STRICT;
CREATE TABLE refund_reservations (
    receipt TEXT PRIMARY KEY REFERENCES mpesa_receipts_ledger(receipt) DEFERRABLE INITIALLY DEFERRED,
    refund_id TEXT NOT NULL REFERENCES refund_requests(id)
) STRICT;
CREATE TRIGGER immutable_refund BEFORE UPDATE ON refund_requests
BEGIN SELECT RAISE(ABORT, 'immutable gated refund'); END;

CREATE TABLE payout_reconcile_attempts (
    payout_id TEXT NOT NULL REFERENCES outbound_payouts(id),
    generation INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('PROBE','STATUS')),
    evidence_gate TEXT,
    outcome TEXT CHECK(outcome IS NULL OR json_valid(outcome)),
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    PRIMARY KEY(payout_id,generation)
) STRICT;
CREATE UNIQUE INDEX one_payout_probe ON payout_reconcile_attempts(payout_id) WHERE kind='PROBE';
CREATE TABLE outbound_status_queries (
    id TEXT PRIMARY KEY,
    payout_id TEXT NOT NULL REFERENCES outbound_payouts(id),
    lookup_kind TEXT NOT NULL CHECK(lookup_kind IN ('TransactionID','OriginalConversationID')),
    lookup_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    originator TEXT UNIQUE,
    conversation TEXT UNIQUE
) STRICT;
CREATE TABLE payout_alerts (
    payout_id TEXT PRIMARY KEY REFERENCES outbound_payouts(id),
    reason TEXT NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX one_outbound_receipt ON outbound_payouts(json_extract(data,'$.final_evidence.outcome.Paid.receipt'))
WHERE state='Paid';

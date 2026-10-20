CREATE TABLE liquidity_policy (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    collection TEXT NOT NULL,
    disbursement TEXT NOT NULL CHECK(collection<>disbursement),
    minimum TEXT NOT NULL,
    max_age INTEGER NOT NULL CHECK(max_age>0),
    poll_interval INTEGER NOT NULL CHECK(poll_interval>0)
) STRICT;
CREATE TABLE balance_queries (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL CHECK(role IN ('collection','disbursement')),
    account TEXT NOT NULL,
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    wire_body TEXT NOT NULL CHECK(json_valid(wire_body)),
    originator TEXT UNIQUE,
    conversation TEXT UNIQUE,
    amount TEXT,
    evidence TEXT,
    applied_at INTEGER
) STRICT;
CREATE INDEX balance_queries_role ON balance_queries(role,sequence);
CREATE TABLE quote_float_reservations (
    quote_id TEXT PRIMARY KEY REFERENCES payment_quotes(id),
    amount TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    payout_id TEXT UNIQUE REFERENCES outbound_payouts(id)
) STRICT;
CREATE TABLE liquidity_alerts (
    role TEXT PRIMARY KEY CHECK(role IN ('collection','disbursement')),
    reason TEXT NOT NULL,
    active INTEGER NOT NULL CHECK(active IN (0,1)),
    updated_at INTEGER NOT NULL
) STRICT;

-- Rebuild admission and its children together, preserving every existing binding.
CREATE TABLE admission_backup AS SELECT * FROM payout_dispatches;
CREATE TABLE observations_backup AS SELECT * FROM payout_dispatch_observations;
CREATE TABLE links_backup AS SELECT * FROM payout_callback_links;
DROP TABLE payout_callback_links;
DROP TABLE payout_dispatch_observations;
DROP TABLE payout_dispatches;
CREATE TABLE payout_dispatches (
    payout_id TEXT PRIMARY KEY NOT NULL REFERENCES outbound_payouts(id),
    quote_id TEXT UNIQUE REFERENCES payment_quotes(id),
    endpoint TEXT NOT NULL,
    command TEXT NOT NULL CHECK(command IN ('BusinessPayment','BusinessBuyGoods','BusinessPayBill','BusinessTransferFromMMFToUtility')),
    CHECK((command='BusinessTransferFromMMFToUtility' AND quote_id IS NULL) OR (command<>'BusinessTransferFromMMFToUtility' AND quote_id IS NOT NULL))
) STRICT;
INSERT INTO payout_dispatches SELECT * FROM admission_backup;
CREATE TRIGGER payout_dispatch_no_update BEFORE UPDATE ON payout_dispatches
BEGIN SELECT RAISE(ABORT, 'immutable payout admission'); END;
CREATE TRIGGER payout_dispatch_no_delete BEFORE DELETE ON payout_dispatches
BEGIN SELECT RAISE(ABORT, 'immutable payout admission'); END;
CREATE TABLE payout_dispatch_observations (
    payout_id TEXT PRIMARY KEY NOT NULL REFERENCES payout_dispatches(payout_id),
    disposition TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK(json_valid(outcome)),
    observed_at INTEGER NOT NULL CHECK(observed_at>=0)
) STRICT;
INSERT INTO payout_dispatch_observations SELECT * FROM observations_backup;
CREATE TABLE payout_callback_links (
    observation_id TEXT PRIMARY KEY NOT NULL REFERENCES callback_inbox(id),
    payout_id TEXT NOT NULL REFERENCES payout_dispatches(payout_id)
) STRICT;
INSERT INTO payout_callback_links SELECT * FROM links_backup;
CREATE INDEX payout_callbacks ON payout_callback_links(payout_id);
DROP TABLE links_backup;
DROP TABLE observations_backup;
DROP TABLE admission_backup;
CREATE TABLE float_sweeps (
    payout_id TEXT PRIMARY KEY REFERENCES outbound_payouts(id),
    query_id TEXT NOT NULL UNIQUE REFERENCES balance_queries(id),
    created_at INTEGER NOT NULL,
    amount TEXT NOT NULL
) STRICT;
CREATE TRIGGER sweep_no_update BEFORE UPDATE ON float_sweeps
BEGIN SELECT RAISE(ABORT, 'immutable sweep admission'); END;
CREATE TRIGGER sweep_no_delete BEFORE DELETE ON float_sweeps
BEGIN SELECT RAISE(ABORT, 'immutable sweep admission'); END;
CREATE TRIGGER balance_query_identity BEFORE UPDATE OF id,role,account,created_at,wire_body ON balance_queries
BEGIN SELECT RAISE(ABORT, 'immutable balance query'); END;
CREATE TRIGGER quote_float_identity BEFORE UPDATE OF quote_id,amount,expires_at ON quote_float_reservations
BEGIN SELECT RAISE(ABORT, 'immutable quote float'); END;
CREATE TRIGGER quote_float_no_delete BEFORE DELETE ON quote_float_reservations
BEGIN SELECT RAISE(ABORT, 'immutable quote float'); END;

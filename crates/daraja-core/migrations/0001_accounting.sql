CREATE TABLE inbound_quotes (
    id TEXT PRIMARY KEY NOT NULL,
    account_ref TEXT NOT NULL UNIQUE CHECK(length(account_ref)=8 AND account_ref NOT GLOB '*[^a-zA-Z0-9]*'),
    state TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 0 CHECK(version>=0),
    data TEXT NOT NULL CHECK(json_valid(data))
) STRICT;
CREATE TABLE stk_attempts (
    id TEXT PRIMARY KEY NOT NULL,
    quote_id TEXT NOT NULL REFERENCES inbound_quotes(id),
    caller_request TEXT NOT NULL UNIQUE,
    merchant_request TEXT UNIQUE,
    checkout_request TEXT UNIQUE,
    next_poll_at INTEGER NOT NULL CHECK(next_poll_at>=0),
    resend_after INTEGER NOT NULL CHECK(resend_after>=0),
    data TEXT NOT NULL CHECK(json_valid(data))
) STRICT;
CREATE TABLE outbound_payouts (
    id TEXT PRIMARY KEY NOT NULL,
    caller_request TEXT NOT NULL UNIQUE,
    provider_originator TEXT UNIQUE,
    provider_conversation TEXT UNIQUE,
    state TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 0 CHECK(version>=0),
    payload_hash BLOB NOT NULL CHECK(length(payload_hash)=32),
    dispatch TEXT NOT NULL CHECK(json_valid(dispatch)),
    wire_body BLOB NOT NULL CHECK(length(wire_body)>0),
    next_check_at INTEGER NOT NULL CHECK(next_check_at>=0),
    manual_after INTEGER NOT NULL CHECK(manual_after>=next_check_at),
    data TEXT NOT NULL CHECK(json_valid(data))
) STRICT;
CREATE TRIGGER immutable_dispatch BEFORE UPDATE OF id,caller_request,payload_hash,dispatch,wire_body ON outbound_payouts
BEGIN SELECT RAISE(ABORT, 'immutable dispatch'); END;
CREATE TABLE request_bindings (
    caller TEXT NOT NULL,
    operation TEXT NOT NULL,
    request_key TEXT NOT NULL,
    payload_hash BLOB NOT NULL CHECK(length(payload_hash)=32),
    operation_id TEXT NOT NULL,
    PRIMARY KEY(caller,operation,request_key)
) STRICT;
CREATE TRIGGER immutable_request BEFORE UPDATE ON request_bindings
BEGIN SELECT RAISE(ABORT, 'immutable request binding'); END;
CREATE TABLE receipt_observations (
    id TEXT PRIMARY KEY NOT NULL,
    receipt TEXT NOT NULL,
    observed_at INTEGER NOT NULL CHECK(observed_at>=0),
    data TEXT NOT NULL CHECK(json_valid(data))
) STRICT;
CREATE INDEX observations_receipt ON receipt_observations(receipt);
CREATE TABLE mpesa_receipts_ledger (
    receipt TEXT PRIMARY KEY NOT NULL CHECK(length(receipt)>0 AND receipt NOT GLOB '*[^A-Z0-9]*'),
    quote_id TEXT REFERENCES inbound_quotes(id),
    state TEXT NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data))
) STRICT;
CREATE INDEX receipts_quote ON mpesa_receipts_ledger(quote_id);
CREATE TRIGGER immutable_allocation BEFORE UPDATE OF receipt,quote_id ON mpesa_receipts_ledger
BEGIN SELECT RAISE(ABORT, 'immutable receipt allocation'); END;
CREATE TABLE unreconciled_deposits (
    receipt TEXT PRIMARY KEY NOT NULL REFERENCES mpesa_receipts_ledger(receipt),
    reason TEXT NOT NULL,
    resolved_at INTEGER,
    resolution_evidence TEXT
) STRICT;
CREATE TABLE settlement_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    quote_id TEXT NOT NULL UNIQUE REFERENCES inbound_quotes(id),
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    data TEXT NOT NULL CHECK(json_valid(data))
) STRICT;

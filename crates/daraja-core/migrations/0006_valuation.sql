CREATE TABLE payment_quotes (
    id TEXT PRIMARY KEY NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data))
) STRICT;
CREATE TRIGGER immutable_payment_quote_update BEFORE UPDATE ON payment_quotes
BEGIN SELECT RAISE(ABORT, 'immutable payment quote'); END;
CREATE TRIGGER immutable_payment_quote_delete BEFORE DELETE ON payment_quotes
BEGIN SELECT RAISE(ABORT, 'immutable payment quote'); END;
CREATE TABLE payout_fee_audit (
    payout_id TEXT PRIMARY KEY NOT NULL REFERENCES outbound_payouts(id),
    actual_fee TEXT CHECK(actual_fee IS NULL OR json_valid(actual_fee)),
    total_spent TEXT CHECK(total_spent IS NULL OR json_valid(total_spent)),
    recorded_at INTEGER NOT NULL CHECK(recorded_at>=0)
) STRICT;
CREATE TRIGGER immutable_payout_fee_update BEFORE UPDATE ON payout_fee_audit
BEGIN SELECT RAISE(ABORT, 'immutable payout fee audit'); END;
CREATE TRIGGER immutable_payout_fee_delete BEFORE DELETE ON payout_fee_audit
BEGIN SELECT RAISE(ABORT, 'immutable payout fee audit'); END;

CREATE UNIQUE INDEX quoted_payout_once ON outbound_payouts(json_extract(dispatch,'$.quote.id'))
WHERE json_extract(dispatch,'$.quote.id') IS NOT NULL;
CREATE TABLE payout_dispatches (
    payout_id TEXT PRIMARY KEY NOT NULL REFERENCES outbound_payouts(id),
    quote_id TEXT NOT NULL UNIQUE REFERENCES payment_quotes(id),
    endpoint TEXT NOT NULL,
    command TEXT NOT NULL CHECK(command IN ('BusinessPayment','BusinessBuyGoods','BusinessPayBill'))
) STRICT;
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
CREATE TABLE payout_callback_links (
    observation_id TEXT PRIMARY KEY NOT NULL REFERENCES callback_inbox(id),
    payout_id TEXT NOT NULL REFERENCES payout_dispatches(payout_id)
) STRICT;
CREATE INDEX payout_callbacks ON payout_callback_links(payout_id);
CREATE INDEX callback_payout_ids ON callback_inbox(
    json_extract(body,'$.Result.OriginatorConversationID'),
    json_extract(body,'$.Result.ConversationID'), endpoint
);

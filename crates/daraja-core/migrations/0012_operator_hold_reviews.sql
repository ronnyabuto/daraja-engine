CREATE TABLE operator_hold_reviews (
    actor TEXT NOT NULL,
    command_id TEXT NOT NULL,
    quote_id TEXT NOT NULL REFERENCES inbound_quotes(id),
    request TEXT NOT NULL CHECK(json_valid(request)),
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    PRIMARY KEY(actor,command_id)
) STRICT;
CREATE TRIGGER operator_hold_reviews_no_update BEFORE UPDATE ON operator_hold_reviews
BEGIN SELECT RAISE(ABORT,'immutable hold review'); END;
CREATE TRIGGER operator_hold_reviews_no_delete BEFORE DELETE ON operator_hold_reviews
BEGIN SELECT RAISE(ABORT,'immutable hold review'); END;

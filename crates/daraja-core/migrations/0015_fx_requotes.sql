CREATE TABLE operator_fx_requotes (
    actor TEXT NOT NULL,
    command_id TEXT NOT NULL,
    original_quote TEXT NOT NULL UNIQUE REFERENCES inbound_quotes(id),
    replacement_quote TEXT NOT NULL UNIQUE REFERENCES inbound_quotes(id),
    request TEXT NOT NULL CHECK(json_valid(request)),
    event TEXT NOT NULL CHECK(json_valid(event)),
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    PRIMARY KEY(actor,command_id)
) STRICT;
CREATE TRIGGER operator_fx_requotes_no_update BEFORE UPDATE ON operator_fx_requotes
BEGIN SELECT RAISE(ABORT,'immutable FX requote'); END;
CREATE TRIGGER operator_fx_requotes_no_delete BEFORE DELETE ON operator_fx_requotes
BEGIN SELECT RAISE(ABORT,'immutable FX requote'); END;

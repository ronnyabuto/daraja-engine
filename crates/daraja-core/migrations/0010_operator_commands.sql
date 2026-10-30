CREATE TABLE operator_commands (
    actor TEXT NOT NULL,
    command_id TEXT NOT NULL,
    subject TEXT NOT NULL REFERENCES outbound_payouts(id),
    reason TEXT NOT NULL,
    evidence_ref TEXT NOT NULL,
    expected_version INTEGER NOT NULL,
    request_hash BLOB NOT NULL CHECK(length(request_hash)=32),
    request TEXT NOT NULL CHECK(json_valid(request)),
    outcome TEXT NOT NULL CHECK(outcome IN ('APPLIED','CONTRADICTORY')),
    result TEXT NOT NULL CHECK(json_valid(result)),
    result_version INTEGER NOT NULL,
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    PRIMARY KEY(actor,command_id)
) STRICT;
CREATE TRIGGER operator_commands_no_update BEFORE UPDATE ON operator_commands
BEGIN SELECT RAISE(ABORT,'immutable operator command'); END;
CREATE TRIGGER operator_commands_no_delete BEFORE DELETE ON operator_commands
BEGIN SELECT RAISE(ABORT,'immutable operator command'); END;

CREATE TABLE operator_verification_retries (
    actor TEXT NOT NULL,
    command_id TEXT NOT NULL,
    job_id TEXT NOT NULL REFERENCES durable_jobs(id),
    request TEXT NOT NULL CHECK(json_valid(request)),
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    PRIMARY KEY(actor,command_id)
) STRICT;
CREATE TRIGGER operator_verification_retries_no_update BEFORE UPDATE ON operator_verification_retries
BEGIN SELECT RAISE(ABORT,'immutable verification retry'); END;
CREATE TRIGGER operator_verification_retries_no_delete BEFORE DELETE ON operator_verification_retries
BEGIN SELECT RAISE(ABORT,'immutable verification retry'); END;

CREATE TABLE callback_inbox (
    id TEXT PRIMARY KEY NOT NULL,
    endpoint TEXT NOT NULL,
    quote_id TEXT REFERENCES inbound_quotes(id),
    received_at INTEGER NOT NULL CHECK(received_at>=0),
    body TEXT NOT NULL CHECK(json_valid(body))
) STRICT;
CREATE TABLE callback_jobs (
    observation_id TEXT PRIMARY KEY NOT NULL REFERENCES callback_inbox(id),
    due_at INTEGER NOT NULL CHECK(due_at>=0),
    completed_at INTEGER
) STRICT;
CREATE INDEX callback_jobs_due ON callback_jobs(completed_at,due_at);

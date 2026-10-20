CREATE TABLE durable_jobs_v4 (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('VERIFY_RECEIPT','RECONCILE_PAYOUT','POLL_STK','INBOUND_CALLBACK')),
    subject TEXT NOT NULL,
    due_at INTEGER NOT NULL CHECK(due_at>=0),
    lease_until INTEGER,
    generation INTEGER NOT NULL DEFAULT 0 CHECK(generation>=0),
    completed_at INTEGER,
    poll_slot INTEGER NOT NULL DEFAULT 0 CHECK(poll_slot>=0),
    retry_count INTEGER NOT NULL DEFAULT 0 CHECK(retry_count>=0),
    last_reason TEXT,
    UNIQUE(kind,subject)
) STRICT;
INSERT INTO durable_jobs_v4(id,kind,subject,due_at,lease_until,generation,completed_at)
SELECT id,kind,subject,due_at,lease_until,generation,completed_at FROM durable_jobs;
DROP TABLE durable_jobs;
ALTER TABLE durable_jobs_v4 RENAME TO durable_jobs;
CREATE INDEX jobs_due ON durable_jobs(completed_at,due_at,lease_until);
INSERT INTO durable_jobs(id,kind,subject,due_at,completed_at)
SELECT 'callback:'||observation_id,'INBOUND_CALLBACK',observation_id,due_at,completed_at
FROM callback_jobs WHERE observation_id IN
(SELECT id FROM callback_inbox WHERE endpoint IN ('/callbacks/stk/{secret}','/callbacks/c2b/confirmation/{secret}'));
CREATE TABLE inbound_status_queries (
    id TEXT PRIMARY KEY NOT NULL,
    receipt TEXT NOT NULL,
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    originator TEXT UNIQUE,
    conversation TEXT UNIQUE
) STRICT;

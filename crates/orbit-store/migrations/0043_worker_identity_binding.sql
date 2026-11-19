-- Bind executions and worker heartbeats to the worker token that claimed/reported them
-- (issue #139). Existing rows keep NULL and are treated as legacy.
ALTER TABLE executions
    ADD COLUMN IF NOT EXISTS worker_token_id uuid;

ALTER TABLE worker_heartbeats
    ADD COLUMN IF NOT EXISTS org_id uuid,
    ADD COLUMN IF NOT EXISTS token_id uuid;

CREATE INDEX IF NOT EXISTS worker_heartbeats_org_idx ON worker_heartbeats (org_id);

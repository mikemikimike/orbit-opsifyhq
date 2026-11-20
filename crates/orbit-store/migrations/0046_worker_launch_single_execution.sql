-- A launch created for exactly one execution. Its worker token is revoked as soon as
-- that execution reaches a terminal state (single-execution tokens never outlive it).
ALTER TABLE worker_launches
    ADD COLUMN IF NOT EXISTS single_execution_id uuid;

CREATE INDEX IF NOT EXISTS worker_launches_single_execution_idx
    ON worker_launches (single_execution_id)
    WHERE single_execution_id IS NOT NULL;

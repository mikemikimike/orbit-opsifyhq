-- #113: atomic team run / queue transitions.
--
-- 1. Lease used by the team-run advance loop so only one API replica advances a
--    run at a time (expires, so a crashed replica never strands a run).
ALTER TABLE team_runs
    ADD COLUMN IF NOT EXISTS advance_claimed_at timestamptz;

-- 2. team_steps already has UNIQUE (run_id, step_index) since 0010.

-- 3. Queue item <-> run lookups (every terminal run) and one-item-per-run guarantee.
CREATE UNIQUE INDEX IF NOT EXISTS team_queue_items_team_run_idx
    ON team_queue_items (team_run_id)
    WHERE team_run_id IS NOT NULL;

-- 4. Status domain. Normalise any unexpected legacy value, then enforce.
UPDATE team_queue_items
SET status = 'failed',
    error = COALESCE(error, 'unknown status normalised by migration 0041')
WHERE status NOT IN ('pending', 'running', 'succeeded', 'failed', 'cancelled');

ALTER TABLE team_queue_items
    ADD CONSTRAINT team_queue_items_status_check
    CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'cancelled'));

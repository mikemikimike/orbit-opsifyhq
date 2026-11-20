-- #121: at most one live long-lived launch per worker profile.
-- Single-execution launches (one per team step) may legitimately coexist, so they are
-- flagged and excluded from the unique index.
ALTER TABLE worker_launches
    ADD COLUMN IF NOT EXISTS single_execution boolean NOT NULL DEFAULT false;

-- Pre-existing duplicate live launches of a profile would make the index creation fail.
-- Keep the newest one as the long-lived launch and exempt the older ones from the index
-- (their status is untouched; they drain and stop as before).
UPDATE worker_launches wl
SET single_execution = true
WHERE wl.archived_at IS NULL
  AND wl.status IN ('pending', 'starting', 'running')
  AND wl.id <> (
      SELECT w2.id FROM worker_launches w2
      WHERE w2.profile_id = wl.profile_id
        AND w2.archived_at IS NULL
        AND w2.status IN ('pending', 'starting', 'running')
      ORDER BY w2.created_at DESC, w2.id DESC
      LIMIT 1
  );

CREATE UNIQUE INDEX IF NOT EXISTS worker_launches_live_profile_uidx
    ON worker_launches (profile_id)
    WHERE single_execution = false
      AND archived_at IS NULL
      AND status IN ('pending', 'starting', 'running');

-- Mark triggers that are derived from an agent version spec ("managed"). Agent version sync
-- only replaces managed rows; triggers created through POST /api/v1/triggers are never
-- touched by it (#111).
ALTER TABLE triggers ADD COLUMN IF NOT EXISTS managed boolean NOT NULL DEFAULT false;

-- Backfill. Nothing recorded the origin before this migration, so infer it: the API handler
-- always stamps `owner_user_id` on the triggers it creates (and may set `harness_kind`),
-- while spec sync inserted rows with neither. Rows with no owner and no harness are
-- therefore treated as spec-managed; everything else is treated as operator-created and
-- is preserved from now on. A spec-synced row that was later given an owner or harness by
-- hand is conservatively kept as unmanaged (it is never deleted by sync).
UPDATE triggers
SET managed = true
WHERE kind <> 'manual'
  AND owner_user_id IS NULL
  AND harness_kind IS NULL;

CREATE INDEX IF NOT EXISTS triggers_managed_agent_idx
    ON triggers (org_id, agent_id) WHERE managed;

-- Output delivery hardening (#112).
--
-- 1. Claim/lease columns so replicas never send the same delivery concurrently and an
--    interrupted send is reclaimed once its lease expires.
-- 2. A terminal `skipped` status (output deleted/disabled) in addition to
--    `failed` (permanent error) and `dead` (retries exhausted).
-- 3. Per-output enqueue bounds replacing the fixed 30-minute look-back:
--    `enqueue_from` is the earliest finish time an output may deliver (creation or last
--    re-enable, so it never back-fills history); `enqueue_cursor` is a high-water mark of
--    the last completed enqueue pass.

ALTER TABLE output_deliveries
    ADD COLUMN IF NOT EXISTS lease_until timestamptz,
    ADD COLUMN IF NOT EXISTS claim_token uuid;

ALTER TABLE output_deliveries DROP CONSTRAINT IF EXISTS output_deliveries_status_check;
ALTER TABLE output_deliveries
    ADD CONSTRAINT output_deliveries_status_check
    CHECK (status IN ('pending', 'sending', 'succeeded', 'failed', 'dead', 'skipped'));

CREATE INDEX IF NOT EXISTS output_deliveries_lease_idx
    ON output_deliveries (lease_until) WHERE status = 'sending';

ALTER TABLE outputs
    ADD COLUMN IF NOT EXISTS enqueue_from timestamptz,
    ADD COLUMN IF NOT EXISTS enqueue_cursor timestamptz;
-- Existing outputs previously considered a 30-minute window; keep parity, no old backfill.
UPDATE outputs SET enqueue_from = now() - interval '30 minutes',
                   enqueue_cursor = now() - interval '30 minutes'
WHERE enqueue_from IS NULL;
ALTER TABLE outputs
    ALTER COLUMN enqueue_from SET DEFAULT now(),
    ALTER COLUMN enqueue_from SET NOT NULL,
    ALTER COLUMN enqueue_cursor SET DEFAULT now(),
    ALTER COLUMN enqueue_cursor SET NOT NULL;

CREATE INDEX IF NOT EXISTS executions_updated_idx ON executions (updated_at);
CREATE INDEX IF NOT EXISTS team_runs_updated_idx ON team_runs (updated_at);

-- Environment mappings are shareable resources with the same ownership/visibility model
-- as agents, connections and worker profiles (ADR 0011). Existing rows stay org-visible.
ALTER TABLE env_mappings
    ADD COLUMN IF NOT EXISTS owner_user_id uuid,
    ADD COLUMN IF NOT EXISTS owner_team_id uuid,
    ADD COLUMN IF NOT EXISTS visibility text NOT NULL DEFAULT 'org';

ALTER TABLE env_mappings DROP CONSTRAINT IF EXISTS env_mappings_visibility_check;
ALTER TABLE env_mappings
    ADD CONSTRAINT env_mappings_visibility_check
    CHECK (visibility IN ('private', 'team', 'org'));

CREATE INDEX IF NOT EXISTS env_mappings_owner_user_idx ON env_mappings (owner_user_id);
CREATE INDEX IF NOT EXISTS env_mappings_owner_team_idx ON env_mappings (owner_team_id);

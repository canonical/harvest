ALTER TABLE artifacts ALTER COLUMN project_id DROP NOT NULL;
ALTER TABLE artifacts ADD COLUMN IF NOT EXISTS user_id text REFERENCES users ON DELETE CASCADE;
ALTER TABLE artifacts DROP CONSTRAINT IF EXISTS artifacts_one_owner;
ALTER TABLE artifacts ADD CONSTRAINT artifacts_one_owner CHECK ((project_id IS NULL) <> (user_id IS NULL));
CREATE INDEX IF NOT EXISTS artifacts_user ON artifacts (user_id, created_at DESC);

DROP INDEX IF EXISTS skills_global_name;
ALTER TABLE skills DROP COLUMN IF EXISTS is_global;
ALTER TABLE skills ADD COLUMN IF NOT EXISTS user_id text REFERENCES users ON DELETE CASCADE;
ALTER TABLE skills ADD COLUMN is_global boolean
    GENERATED ALWAYS AS (project_id IS NULL AND user_id IS NULL) STORED;
ALTER TABLE skills DROP CONSTRAINT IF EXISTS skills_at_most_one_owner;
ALTER TABLE skills ADD CONSTRAINT skills_at_most_one_owner CHECK (project_id IS NULL OR user_id IS NULL);
CREATE UNIQUE INDEX skills_global_name ON skills (name) WHERE is_global;
CREATE UNIQUE INDEX IF NOT EXISTS skills_user_name ON skills (user_id, name) WHERE user_id IS NOT NULL;

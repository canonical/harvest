DROP INDEX IF EXISTS conversations_code_session;
CREATE INDEX IF NOT EXISTS conversations_by_code_session ON conversations (code_session_id) WHERE code_session_id IS NOT NULL;

ALTER TABLE code_sessions ADD COLUMN IF NOT EXISTS layout jsonb;

ALTER TABLE code_previews DROP CONSTRAINT IF EXISTS code_previews_session_id_key;
CREATE INDEX IF NOT EXISTS code_previews_by_session ON code_previews (session_id);
ALTER TABLE code_previews ADD COLUMN IF NOT EXISTS conversation_id text REFERENCES conversations ON DELETE SET NULL;
ALTER TABLE code_previews ADD COLUMN IF NOT EXISTS last_used_at timestamptz NOT NULL DEFAULT now();

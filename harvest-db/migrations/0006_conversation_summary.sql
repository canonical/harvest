ALTER TABLE conversations ADD COLUMN IF NOT EXISTS summary text;
ALTER TABLE conversations ADD COLUMN IF NOT EXISTS summary_upto integer NOT NULL DEFAULT 0;

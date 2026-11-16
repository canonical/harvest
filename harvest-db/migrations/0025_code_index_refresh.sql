ALTER TABLE code_projects ADD COLUMN IF NOT EXISTS index_checked_at timestamptz;
ALTER TABLE code_projects ADD COLUMN IF NOT EXISTS index_error text;

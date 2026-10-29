ALTER TABLE conversations ADD COLUMN IF NOT EXISTS summary_lease_until timestamptz;

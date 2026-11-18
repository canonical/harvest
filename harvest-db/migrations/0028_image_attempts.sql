ALTER TABLE code_images DROP CONSTRAINT IF EXISTS code_images_status_check;

ALTER TABLE code_images
    ADD COLUMN IF NOT EXISTS attempt_id      text,
    ADD COLUMN IF NOT EXISTS attempt_status  text CHECK (attempt_status IN ('building', 'failed')),
    ADD COLUMN IF NOT EXISTS attempt_failure text CHECK (attempt_failure IN ('lxd_unavailable', 'infrastructure')),
    ADD COLUMN IF NOT EXISTS attempt_hash    text,
    ADD COLUMN IF NOT EXISTS attempt_log     text NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS attempted_at    timestamptz,
    ADD COLUMN IF NOT EXISTS setup_warning   text;

UPDATE code_images SET
    attempt_status  = CASE WHEN status IN ('building', 'failed') THEN status END,
    attempt_failure = CASE WHEN status = 'failed' THEN 'infrastructure' END,
    attempt_hash    = setup_hash,
    attempt_log     = CASE WHEN status = 'failed' THEN log ELSE '' END,
    attempted_at    = built_at,
    status          = CASE WHEN status = 'ready' THEN 'ready' ELSE 'none' END
WHERE status <> 'ready' OR attempted_at IS NULL;

ALTER TABLE code_images ADD CONSTRAINT code_images_status_check CHECK (status IN ('ready', 'none'));

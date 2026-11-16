ALTER TABLE repositories ADD COLUMN IF NOT EXISTS host text;
ALTER TABLE repositories ADD COLUMN IF NOT EXISTS visibility text NOT NULL DEFAULT 'public';
ALTER TABLE repositories ADD COLUMN IF NOT EXISTS default_branch text;
ALTER TABLE repositories DROP CONSTRAINT IF EXISTS repositories_visibility_check;
ALTER TABLE repositories ADD CONSTRAINT repositories_visibility_check
    CHECK (visibility IN ('public', 'private'));

CREATE TABLE IF NOT EXISTS repository_access (
    repository_id bigint      NOT NULL REFERENCES repositories ON DELETE CASCADE,
    user_id       text        NOT NULL REFERENCES users ON DELETE CASCADE,
    verified_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (repository_id, user_id)
);
CREATE INDEX IF NOT EXISTS repository_access_user ON repository_access (user_id);

CREATE TABLE IF NOT EXISTS ssh_keys (
    user_id                text PRIMARY KEY REFERENCES users ON DELETE CASCADE,
    private_key_ciphertext text        NOT NULL,
    private_key_nonce      text        NOT NULL,
    public_key             text        NOT NULL,
    fingerprint            text        NOT NULL,
    created_at             timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS platform_accounts (
    user_id           text        NOT NULL REFERENCES users ON DELETE CASCADE,
    platform          text        NOT NULL,
    host              text        NOT NULL,
    username          text        NOT NULL DEFAULT '',
    secret_ciphertext text        NOT NULL,
    secret_nonce      text        NOT NULL,
    verified_at       timestamptz,
    created_at        timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, host),
    CHECK (platform IN ('github', 'gitlab', 'gitea', 'launchpad', 'gerrit'))
);

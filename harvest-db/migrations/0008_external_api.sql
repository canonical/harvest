CREATE TABLE user_api_keys (
    id          text PRIMARY KEY,
    user_id     text NOT NULL REFERENCES users ON DELETE CASCADE,
    name        text NOT NULL,
    key_prefix  text NOT NULL UNIQUE,
    key_hash    text NOT NULL,
    created_at  timestamptz NOT NULL,
    updated_at  timestamptz NOT NULL,
    last_used_at timestamptz,
    revoked_at  timestamptz
);
CREATE INDEX user_api_keys_user ON user_api_keys (user_id, created_at DESC);

ALTER TABLE conversations
    ADD COLUMN IF NOT EXISTS origin_type text NOT NULL DEFAULT 'web',
    ADD COLUMN IF NOT EXISTS origin_api_key_id text REFERENCES user_api_keys ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS origin_tracking_id text;

ALTER TABLE conversations
    ADD CONSTRAINT conversations_origin_type_check
    CHECK (origin_type IN ('web', 'api'));

CREATE INDEX IF NOT EXISTS conversations_origin_api_key ON conversations (origin_api_key_id, updated_at DESC);

CREATE TABLE api_chat_threads (
    id              text PRIMARY KEY,
    api_key_id      text NOT NULL REFERENCES user_api_keys ON DELETE CASCADE,
    tracking_id     text NOT NULL,
    conversation_id text NOT NULL REFERENCES conversations ON DELETE CASCADE,
    created_at      timestamptz NOT NULL,
    updated_at      timestamptz NOT NULL,
    UNIQUE (api_key_id, tracking_id)
);
CREATE INDEX api_chat_threads_conversation ON api_chat_threads (conversation_id);

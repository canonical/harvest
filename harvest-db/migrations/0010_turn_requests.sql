CREATE TABLE turn_requests (
    user_id    text NOT NULL,
    key        text NOT NULL,
    conv_id    text NOT NULL,
    turn_id    text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, key)
);
CREATE INDEX turn_requests_created ON turn_requests (created_at);

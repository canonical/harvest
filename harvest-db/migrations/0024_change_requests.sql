CREATE TABLE code_change_requests (
    session_id text        PRIMARY KEY REFERENCES code_sessions ON DELETE CASCADE,
    platform   text        NOT NULL,
    number     text        NOT NULL,
    url        text        NOT NULL,
    state      text        NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'merged', 'closed')),
    created_at timestamptz NOT NULL DEFAULT now(),
    checked_at timestamptz NOT NULL DEFAULT now()
);

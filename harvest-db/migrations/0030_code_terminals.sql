CREATE TABLE code_terminals (
    id               text        PRIMARY KEY,
    session_id       text        NOT NULL REFERENCES code_sessions ON DELETE CASCADE,
    slot             smallint    NOT NULL CHECK (slot BETWEEN 1 AND 8),
    title            text        NOT NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    last_attached_at timestamptz,
    UNIQUE (session_id, slot)
);

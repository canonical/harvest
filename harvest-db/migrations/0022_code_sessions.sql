CREATE TABLE code_projects (
    id            text        PRIMARY KEY,
    repository_id bigint      NOT NULL UNIQUE REFERENCES repositories ON DELETE CASCADE,
    setup_script  text,
    created_by    text,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_used_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE code_images (
    project_id  text        PRIMARY KEY REFERENCES code_projects ON DELETE CASCADE,
    alias       text        NOT NULL,
    status      text        NOT NULL CHECK (status IN ('building', 'ready', 'failed')),
    base_commit text,
    setup_hash  text        NOT NULL DEFAULT '',
    log         text        NOT NULL DEFAULT '',
    built_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE code_containers (
    name           text        PRIMARY KEY,
    project_id     text        NOT NULL REFERENCES code_projects ON DELETE CASCADE,
    role           text        NOT NULL CHECK (role IN ('pool', 'session', 'preview')),
    state          text        NOT NULL CHECK (state IN ('building', 'ready', 'claimed', 'failed')),
    base_commit    text,
    created_at     timestamptz NOT NULL DEFAULT now(),
    last_active_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX code_containers_pool ON code_containers (project_id, role, state);

CREATE TABLE code_sessions (
    id             text        PRIMARY KEY,
    project_id     text        NOT NULL REFERENCES code_projects ON DELETE CASCADE,
    user_id        text        NOT NULL REFERENCES users ON DELETE CASCADE,
    branch         text        NOT NULL,
    base_branch    text        NOT NULL,
    container      text,
    status         text        NOT NULL CHECK (status IN ('starting', 'active', 'closed', 'failed')),
    close_reason   text        CHECK (close_reason IN ('merged', 'idle', 'manual')),
    error          text,
    created_at     timestamptz NOT NULL DEFAULT now(),
    last_active_at timestamptz NOT NULL DEFAULT now(),
    closed_at      timestamptz
);
CREATE UNIQUE INDEX code_sessions_one_open_branch
    ON code_sessions (project_id, user_id, branch) WHERE status IN ('starting', 'active');
CREATE INDEX code_sessions_user ON code_sessions (user_id, created_at DESC);

ALTER TABLE conversations ADD COLUMN code_session_id text REFERENCES code_sessions ON DELETE CASCADE;
CREATE UNIQUE INDEX conversations_code_session ON conversations (code_session_id) WHERE code_session_id IS NOT NULL;

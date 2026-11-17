CREATE TABLE code_session_jobs (
    id          text        PRIMARY KEY,
    session_id  text        NOT NULL REFERENCES code_sessions ON DELETE CASCADE,
    chat_id     text,
    kind        text        NOT NULL CHECK (kind IN ('prepare', 'command', 'checks', 'review')),
    label       text        NOT NULL,
    status      text        NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'cancelled')),
    exit_code   bigint,
    output      text        NOT NULL DEFAULT '',
    reported    boolean     NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz
);
CREATE INDEX code_session_jobs_session ON code_session_jobs (session_id, created_at DESC);
CREATE INDEX code_session_jobs_running ON code_session_jobs (status) WHERE status = 'running';

ALTER TABLE code_projects ADD COLUMN IF NOT EXISTS checks text;

CREATE TABLE code_learned_setup (
    project_id text        NOT NULL REFERENCES code_projects ON DELETE CASCADE,
    command    text        NOT NULL,
    learned_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, command)
);

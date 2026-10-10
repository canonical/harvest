CREATE TABLE cluster_nodes (
    node_id      text PRIMARY KEY,
    internal_url text NOT NULL,
    version      text NOT NULL DEFAULT '',
    started_at   timestamptz NOT NULL DEFAULT now(),
    heartbeat_at timestamptz NOT NULL DEFAULT now(),
    draining     boolean NOT NULL DEFAULT false
);

CREATE UNLOGGED TABLE bus_payloads (
    id         bigserial PRIMARY KEY,
    body       text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE active_turns (
    conv_id     text PRIMARY KEY,
    project_id  text NOT NULL,
    turn_id     text NOT NULL,
    node_id     text NOT NULL,
    locked_by   text NOT NULL,
    query       text NOT NULL DEFAULT '',
    attachments jsonb NOT NULL DEFAULT '[]',
    started_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX active_turns_project ON active_turns (project_id);
CREATE INDEX active_turns_node ON active_turns (node_id);

CREATE TABLE paused_turns (
    conv_id    text PRIMARY KEY,
    project_id text NOT NULL,
    payload    jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE project_presence (
    connection_id text PRIMARY KEY,
    project_id    text NOT NULL,
    user_id       text NOT NULL,
    name          text NOT NULL,
    conv_id       text,
    node_id       text NOT NULL,
    connected_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX project_presence_project ON project_presence (project_id);
CREATE INDEX project_presence_node ON project_presence (node_id);

CREATE TABLE auth_ephemeral (
    key        text PRIMARY KEY,
    kind       text NOT NULL,
    data       jsonb NOT NULL,
    expires_at timestamptz NOT NULL
);
CREATE INDEX auth_ephemeral_expiry ON auth_ephemeral (expires_at);

CREATE TABLE agent_connections (
    agent_id      text PRIMARY KEY,
    node_id       text NOT NULL,
    connection_id text NOT NULL,
    project_id    text NOT NULL,
    hostname      text NOT NULL,
    connected_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX agent_connections_project ON agent_connections (project_id);
CREATE INDEX agent_connections_node ON agent_connections (node_id);

CREATE TABLE ingestion_jobs (
    id          text PRIMARY KEY,
    repo        text NOT NULL,
    kind        text NOT NULL,
    status      text NOT NULL,
    node_id     text NOT NULL,
    error       text,
    versions    jsonb,
    started_at  timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz
);
CREATE UNIQUE INDEX ingestion_jobs_one_active ON ingestion_jobs (repo) WHERE status IN ('pending', 'running');
CREATE INDEX ingestion_jobs_repo ON ingestion_jobs (repo, started_at DESC);

CREATE TABLE ingestion_progress (
    seq    bigserial PRIMARY KEY,
    job_id text NOT NULL REFERENCES ingestion_jobs ON DELETE CASCADE,
    line   text NOT NULL
);
CREATE INDEX ingestion_progress_job ON ingestion_progress (job_id, seq);

CREATE TABLE collocate_sessions (
    container_id    text PRIMARY KEY,
    name            text NOT NULL,
    project_id      text NOT NULL,
    conversation_id text NOT NULL,
    node_id         text NOT NULL,
    persistent      boolean NOT NULL DEFAULT false,
    address         text,
    published       jsonb NOT NULL DEFAULT '[]',
    created_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX collocate_sessions_conversation ON collocate_sessions (project_id, conversation_id);

ALTER TABLE design_pdf_cache ADD COLUMN IF NOT EXISTS generating_node text;
ALTER TABLE design_pdf_cache ADD COLUMN IF NOT EXISTS generating_until timestamptz;

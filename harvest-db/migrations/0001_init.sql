CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- ───────────── Code graph (written by knowledge-harvester) ─────────────

CREATE TABLE repositories (
    id   bigserial PRIMARY KEY,
    name text NOT NULL UNIQUE,
    url  text
);

CREATE TABLE versions (
    id            bigserial PRIMARY KEY,
    repository_id bigint  NOT NULL REFERENCES repositories ON DELETE CASCADE,
    tag           text    NOT NULL,
    commit_sha    text,
    timestamp     bigint,
    ingested      boolean NOT NULL DEFAULT false,
    UNIQUE (repository_id, tag)
);

CREATE TABLE files (
    id         bigserial PRIMARY KEY,
    version_id bigint NOT NULL REFERENCES versions ON DELETE CASCADE,
    path       text   NOT NULL,
    language   text,
    UNIQUE (version_id, path)
);
CREATE INDEX files_path_trgm ON files USING gin (path gin_trgm_ops);

CREATE TABLE symbols (
    id         bigserial PRIMARY KEY,
    file_id    bigint NOT NULL REFERENCES files ON DELETE CASCADE,
    version_id bigint NOT NULL REFERENCES versions ON DELETE CASCADE,
    label      text   NOT NULL CHECK (label IN ('Function', 'Class')),
    name       text   NOT NULL,
    kind       text,
    signature  text,
    start_line integer,
    end_line   integer,
    source     text,
    impl_type  text,
    bases      text[],
    traits     text[],
    embeds     text[],
    uses       text[],
    UNIQUE (file_id, label, name)
);
CREATE INDEX symbols_by_name   ON symbols (version_id, label, name);
CREATE INDEX symbols_name_trgm ON symbols USING gin (lower(name) gin_trgm_ops);

CREATE TABLE imports (
    id      bigserial PRIMARY KEY,
    file_id bigint NOT NULL REFERENCES files ON DELETE CASCADE,
    target  text   NOT NULL,
    line    integer,
    UNIQUE (file_id, target)
);

CREATE TABLE symbol_edges (
    src_id   bigint NOT NULL REFERENCES symbols ON DELETE CASCADE,
    dst_id   bigint NOT NULL REFERENCES symbols ON DELETE CASCADE,
    relation text   NOT NULL CHECK (relation IN ('CALLS', 'INHERITS', 'IMPLEMENTS', 'USES', 'EMBEDS')),
    line     integer,
    UNIQUE NULLS NOT DISTINCT (src_id, dst_id, relation, line)
);
CREATE INDEX symbol_edges_dst ON symbol_edges (dst_id, relation);

CREATE VIEW code_repositories AS
    SELECT r.id, r.name, r.url FROM repositories r;

CREATE VIEW code_versions AS
    SELECT v.id, v.repository_id, r.name AS repo, v.tag, v.commit_sha, v.timestamp, v.ingested
    FROM versions v JOIN repositories r ON r.id = v.repository_id;

CREATE VIEW code_files AS
    SELECT f.id, f.version_id, r.name AS repo, v.tag AS version, f.path, f.language
    FROM files f
    JOIN versions v     ON v.id = f.version_id
    JOIN repositories r ON r.id = v.repository_id;

CREATE VIEW code_symbols AS
    SELECT s.id, s.file_id, s.version_id, r.name AS repo, v.tag AS version, f.path AS file,
           s.label, s.name, s.kind, s.signature, s.start_line, s.end_line, s.source,
           s.impl_type, s.bases, s.traits, s.embeds, s.uses
    FROM symbols s
    JOIN files f        ON f.id = s.file_id
    JOIN versions v     ON v.id = s.version_id
    JOIN repositories r ON r.id = v.repository_id;

CREATE VIEW code_imports AS
    SELECT i.id, i.file_id, f.repo, f.version, f.path AS file, i.target, i.line
    FROM imports i JOIN code_files f ON f.id = i.file_id;

CREATE VIEW code_edges AS
    SELECT e.relation, e.line, a.repo, a.version,
           a.id AS src_id, a.label AS src_label, a.file AS src_file, a.name AS src_name,
           b.id AS dst_id, b.label AS dst_label, b.file AS dst_file, b.name AS dst_name
    FROM symbol_edges e
    JOIN code_symbols a ON a.id = e.src_id
    JOIN code_symbols b ON b.id = e.dst_id;

-- The LLM's run_sql tool executes as this role, which can only read the code_* views.
-- Creating it needs CREATEROLE; without it run_sql reports itself unavailable.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'harvest_graph_reader') THEN
        CREATE ROLE harvest_graph_reader NOLOGIN;
    END IF;
EXCEPTION WHEN insufficient_privilege THEN
    RAISE NOTICE 'harvest_graph_reader role not created: %', SQLERRM;
END $$;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'harvest_graph_reader') THEN
        GRANT SELECT ON code_repositories, code_versions, code_files, code_symbols,
                        code_imports, code_edges TO harvest_graph_reader;
    END IF;
END $$;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'harvest_graph_reader')
       AND NOT pg_has_role(current_user, 'harvest_graph_reader', 'SET') THEN
        EXECUTE format('GRANT harvest_graph_reader TO %I', current_user);
    END IF;
EXCEPTION WHEN insufficient_privilege OR invalid_grant_operation THEN
    RAISE NOTICE 'could not grant harvest_graph_reader to %: %', current_user, SQLERRM;
END $$;

-- ───────────── Application data (written by knowledge-server) ─────────────

CREATE TABLE users (
    id                   text PRIMARY KEY,
    email                text UNIQUE,
    name                 text,
    password_hash        text,
    provider             text NOT NULL,
    role                 text NOT NULL DEFAULT 'regular',
    google_id            text UNIQUE,
    oidc_sub             text UNIQUE,
    last_project_id      text,
    last_llm_provider_id text,
    last_llm_model       text,
    created_at           timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE groups (
    id          text PRIMARY KEY,
    name        text NOT NULL,
    description text NOT NULL DEFAULT '',
    is_default  boolean NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE user_groups (
    user_id  text NOT NULL REFERENCES users  ON DELETE CASCADE,
    group_id text NOT NULL REFERENCES groups ON DELETE CASCADE,
    PRIMARY KEY (user_id, group_id)
);
CREATE INDEX user_groups_group ON user_groups (group_id);

CREATE TABLE user_llm_keys (
    user_id        text NOT NULL REFERENCES users ON DELETE CASCADE,
    provider_id    text NOT NULL,
    key_ciphertext text NOT NULL,
    key_nonce      text NOT NULL,
    updated_at     timestamptz NOT NULL,
    PRIMARY KEY (user_id, provider_id)
);

CREATE TABLE projects (
    id            text PRIMARY KEY,
    group_id      text NOT NULL REFERENCES groups ON DELETE RESTRICT,
    name          text NOT NULL,
    description   text,
    created_by    text,
    created_at    timestamptz NOT NULL,
    install_token text UNIQUE
);
CREATE INDEX projects_group ON projects (group_id);

CREATE TABLE conversations (
    id            text PRIMARY KEY,
    user_id       text REFERENCES users    ON DELETE CASCADE,
    project_id    text REFERENCES projects ON DELETE CASCADE,
    title         text,
    messages      text    NOT NULL DEFAULT '[]',
    message_count integer NOT NULL DEFAULT 0,
    created_by    text,
    created_at    timestamptz NOT NULL,
    updated_at    timestamptz NOT NULL,
    CHECK ((user_id IS NULL) <> (project_id IS NULL))
);
CREATE INDEX conversations_user    ON conversations (user_id, updated_at DESC);
CREATE INDEX conversations_project ON conversations (project_id, updated_at DESC);

CREATE TABLE chat_layouts (
    id         text PRIMARY KEY,
    user_id    text NOT NULL REFERENCES users ON DELETE CASCADE,
    project_id text,
    kind       text NOT NULL CHECK (kind IN ('current', 'named')),
    name       text,
    tree       text NOT NULL,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL
);
CREATE UNIQUE INDEX chat_layouts_one_current
    ON chat_layouts (user_id, project_id) NULLS NOT DISTINCT WHERE kind = 'current';
CREATE INDEX chat_layouts_user ON chat_layouts (user_id, project_id, kind);

CREATE TABLE artifacts (
    id         text PRIMARY KEY,
    project_id text NOT NULL REFERENCES projects ON DELETE CASCADE,
    title      text NOT NULL,
    kind       text NOT NULL,
    content    text NOT NULL,
    created_by text,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL
);
CREATE INDEX artifacts_project ON artifacts (project_id, created_at DESC);

CREATE TABLE skills (
    id          text PRIMARY KEY,
    project_id  text REFERENCES projects ON DELETE CASCADE,
    is_global   boolean GENERATED ALWAYS AS (project_id IS NULL) STORED,
    name        text NOT NULL,
    description text,
    content     text NOT NULL,
    created_by  text,
    created_at  timestamptz NOT NULL,
    updated_at  timestamptz NOT NULL
);
CREATE UNIQUE INDEX skills_global_name  ON skills (name) WHERE project_id IS NULL;
CREATE UNIQUE INDEX skills_project_name ON skills (project_id, name) WHERE project_id IS NOT NULL;

CREATE TABLE app_flags (
    key    text PRIMARY KEY,
    set_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE product_templates (
    id          text PRIMARY KEY,
    name        text NOT NULL,
    description text,
    content     text NOT NULL,
    created_by  text,
    created_at  timestamptz NOT NULL,
    updated_at  timestamptz NOT NULL
);

CREATE TABLE deployments (
    id                       text PRIMARY KEY,
    project_id               text NOT NULL REFERENCES projects ON DELETE CASCADE,
    template_id              text REFERENCES product_templates ON DELETE SET NULL,
    design_doc_id            text REFERENCES artifacts ON DELETE SET NULL,
    terraform_bundle_id      text REFERENCES artifacts ON DELETE SET NULL,
    guide_id                 text REFERENCES artifacts ON DELETE SET NULL,
    name                     text NOT NULL,
    environment_description  text NOT NULL DEFAULT '',
    infra_state              text NOT NULL DEFAULT 'none',
    last_applied_content     text,
    last_applied_artifact_id text,
    last_applied_at          timestamptz,
    created_by               text,
    created_at               timestamptz NOT NULL,
    updated_at               timestamptz NOT NULL
);
CREATE INDEX deployments_project  ON deployments (project_id, updated_at DESC);
CREATE INDEX deployments_template ON deployments (template_id);

CREATE TABLE deployment_context_artifacts (
    deployment_id text NOT NULL REFERENCES deployments ON DELETE CASCADE,
    artifact_id   text NOT NULL REFERENCES artifacts   ON DELETE CASCADE,
    PRIMARY KEY (deployment_id, artifact_id)
);

CREATE TABLE design_pdf_cache (
    deployment_id              text PRIMARY KEY REFERENCES deployments ON DELETE CASCADE,
    ready_artifact_id          text,
    ready_artifact_updated_at  text,
    ready_bytes                text,
    ready_generated_at         timestamptz,
    failed_artifact_id         text,
    failed_artifact_updated_at text,
    failed_error               text
);

CREATE TABLE deployment_runs (
    id             text PRIMARY KEY,
    deployment_id  text NOT NULL REFERENCES deployments ON DELETE CASCADE,
    action         text,
    status         text,
    exit_code      integer,
    stdout_preview text,
    stderr_preview text,
    step_id        text,
    artifact_id    text,
    initiated_by   text,
    reasoning      text,
    created_at     timestamptz NOT NULL
);
CREATE INDEX deployment_runs_deployment ON deployment_runs (deployment_id, created_at DESC);

CREATE TABLE execution_steps (
    id            text PRIMARY KEY,
    deployment_id text NOT NULL REFERENCES deployments ON DELETE CASCADE,
    phase         text NOT NULL,
    action        text NOT NULL,
    label         text,
    step_index    integer NOT NULL DEFAULT 0,
    artifact_id   text REFERENCES artifacts ON DELETE SET NULL,
    created_at    timestamptz NOT NULL
);
CREATE INDEX execution_steps_deployment ON execution_steps (deployment_id, phase, step_index);

CREATE TABLE execution_step_deps (
    step_id       text NOT NULL REFERENCES execution_steps ON DELETE CASCADE,
    depends_on_id text NOT NULL REFERENCES execution_steps ON DELETE CASCADE,
    PRIMARY KEY (step_id, depends_on_id)
);

CREATE TABLE proposals (
    id                 text PRIMARY KEY,
    deployment_id      text NOT NULL REFERENCES deployments ON DELETE CASCADE,
    target_artifact_id text NOT NULL REFERENCES artifacts   ON DELETE CASCADE,
    source             text,
    explanation        text,
    current_content    text,
    proposed_content   text,
    status             text NOT NULL,
    created_at         timestamptz NOT NULL
);
CREATE INDEX proposals_deployment ON proposals (deployment_id, status, created_at DESC);

CREATE TABLE llm_calls (
    id                    text PRIMARY KEY,
    scope                 text NOT NULL,
    turn_id               text,
    project_id            text,
    conversation_id       text REFERENCES conversations ON DELETE CASCADE,
    deployment_id         text,
    proposal_id           text,
    artifact_id           text,
    user_id               text,
    provider_id           text,
    kind                  text,
    model                 text,
    input_tokens          bigint  NOT NULL DEFAULT 0,
    output_tokens         bigint  NOT NULL DEFAULT 0,
    cache_read_tokens     bigint  NOT NULL DEFAULT 0,
    cache_creation_tokens bigint  NOT NULL DEFAULT 0,
    reasoning_tokens      bigint  NOT NULL DEFAULT 0,
    cost_microusd         bigint  NOT NULL DEFAULT 0,
    duration_ms           bigint,
    iteration_index       integer,
    llm_call_count        integer,
    succeeded             boolean NOT NULL,
    created_at            timestamptz NOT NULL
);
CREATE INDEX llm_calls_project      ON llm_calls (project_id);
CREATE INDEX llm_calls_conversation ON llm_calls (conversation_id, created_at);
CREATE INDEX llm_calls_deployment   ON llm_calls (deployment_id, created_at DESC);

CREATE TABLE machines (
    id               text PRIMARY KEY,
    project_id       text NOT NULL REFERENCES projects ON DELETE CASCADE,
    hostname         text,
    agent_token_hash text UNIQUE,
    provider         text,
    lxd_instance     text,
    description      text,
    created_at       timestamptz NOT NULL,
    last_seen        timestamptz
);
CREATE INDEX machines_project ON machines (project_id, created_at);

CREATE TABLE lxd_pending_instances (
    project_id  text NOT NULL REFERENCES projects ON DELETE CASCADE,
    hostname    text NOT NULL,
    lxd_project text,
    description text,
    created_at  timestamptz NOT NULL,
    PRIMARY KEY (project_id, hostname)
);

CREATE TABLE port_forwards (
    id         text PRIMARY KEY,
    project_id text    NOT NULL REFERENCES projects ON DELETE CASCADE,
    agent_id   text    NOT NULL REFERENCES machines ON DELETE CASCADE,
    port       integer NOT NULL,
    route_name text    NOT NULL,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    UNIQUE (agent_id, route_name)
);

CREATE TABLE lxd_identity (
    id          text PRIMARY KEY,
    client_cert text NOT NULL,
    client_key  text NOT NULL,
    trusted     boolean NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL
);

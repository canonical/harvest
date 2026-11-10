CREATE TABLE IF NOT EXISTS work_queue (
    id         bigserial PRIMARY KEY,
    kind       text        NOT NULL,
    key        text        NOT NULL,
    payload    jsonb       NOT NULL DEFAULT '{}',
    status     text        NOT NULL DEFAULT 'pending'
               CHECK (status IN ('pending', 'running', 'done', 'failed')),
    claimed_by text,
    claimed_at timestamptz,
    attempts   integer     NOT NULL DEFAULT 0,
    error      text,
    created_at timestamptz NOT NULL DEFAULT now(),
    done_at    timestamptz,
    UNIQUE (kind, key)
);
CREATE INDEX IF NOT EXISTS work_queue_claimable ON work_queue (kind, status, id);

CREATE TABLE IF NOT EXISTS symbol_enrichments (
    symbol_id     bigint PRIMARY KEY REFERENCES symbols ON DELETE CASCADE,
    source_hash   text        NOT NULL,
    model         text        NOT NULL,
    summary       text        NOT NULL,
    questions     text[]      NOT NULL DEFAULT '{}',
    tags          text[]      NOT NULL DEFAULT '{}',
    fencing_token bigint,
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS evidence_cache (
    id            bigserial PRIMARY KEY,
    repo          text        NOT NULL,
    version       text        NOT NULL,
    question      text        NOT NULL,
    evidence      jsonb       NOT NULL,
    symbol_hashes jsonb       NOT NULL DEFAULT '{}',
    source        text        NOT NULL,
    hits          integer     NOT NULL DEFAULT 0,
    created_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repo, version, question)
);
CREATE INDEX IF NOT EXISTS evidence_cache_scope ON evidence_cache (repo, version);

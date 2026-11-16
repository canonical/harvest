CREATE TABLE IF NOT EXISTS search_traces (
    id              bigserial PRIMARY KEY,
    turn_id         text,
    conversation_id text,
    node_id         text,
    engine          text             NOT NULL,
    repo            text,
    version         text,
    query           text             NOT NULL,
    tier_reached    text,
    degraded        text,
    generation      text,
    total_ms        double precision,
    llm_calls       integer          NOT NULL DEFAULT 0,
    shadow_recall   double precision,
    evidence        jsonb            NOT NULL DEFAULT '[]',
    gaps            jsonb            NOT NULL DEFAULT '[]',
    trace           jsonb            NOT NULL DEFAULT '{}',
    created_at      timestamptz      NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS search_traces_created ON search_traces (created_at DESC);
CREATE INDEX IF NOT EXISTS search_traces_scope ON search_traces (repo, version, created_at DESC);

ALTER TABLE symbols DROP CONSTRAINT IF EXISTS symbols_file_id_label_name_key;
CREATE UNIQUE INDEX IF NOT EXISTS symbols_identity
    ON symbols (file_id, label, coalesce(impl_type, ''), name);

ALTER TABLE symbol_edges DROP CONSTRAINT IF EXISTS symbol_edges_relation_check;
ALTER TABLE symbol_edges ADD CONSTRAINT symbol_edges_relation_check CHECK (relation IN (
    'CALLS', 'INHERITS', 'IMPLEMENTS', 'USES', 'EMBEDS', 'OVERRIDES', 'REGISTERS', 'READS_CONFIG',
    'INSTANTIATES', 'RAISES', 'TESTS', 'DOCUMENTS', 'CO_CHANGES'
));
CREATE INDEX IF NOT EXISTS symbol_edges_src ON symbol_edges (src_id, relation);

CREATE TABLE IF NOT EXISTS code_units (
    id           bigserial PRIMARY KEY,
    version_id   bigint NOT NULL REFERENCES versions ON DELETE CASCADE,
    file_id      bigint NOT NULL REFERENCES files ON DELETE CASCADE,
    kind         text   NOT NULL CHECK (kind IN ('module', 'config', 'doc', 'chunk')),
    name         text   NOT NULL,
    path         text   NOT NULL,
    language     text,
    start_line   integer NOT NULL DEFAULT 1,
    end_line     integer NOT NULL DEFAULT 1,
    text         text   NOT NULL,
    symbol_id    bigint REFERENCES symbols ON DELETE CASCADE,
    content_hash text   NOT NULL
);
CREATE INDEX IF NOT EXISTS code_units_version ON code_units (version_id, kind);
CREATE INDEX IF NOT EXISTS code_units_name ON code_units (version_id, name);

CREATE TABLE IF NOT EXISTS unit_edges (
    src_unit_id   bigint REFERENCES code_units ON DELETE CASCADE,
    src_symbol_id bigint REFERENCES symbols ON DELETE CASCADE,
    dst_unit_id   bigint REFERENCES code_units ON DELETE CASCADE,
    dst_symbol_id bigint REFERENCES symbols ON DELETE CASCADE,
    relation      text   NOT NULL,
    weight        real   NOT NULL DEFAULT 1,
    CHECK ((src_unit_id IS NULL) <> (src_symbol_id IS NULL)),
    CHECK ((dst_unit_id IS NULL) <> (dst_symbol_id IS NULL))
);
CREATE INDEX IF NOT EXISTS unit_edges_src_unit ON unit_edges (src_unit_id);
CREATE INDEX IF NOT EXISTS unit_edges_src_symbol ON unit_edges (src_symbol_id);

CREATE OR REPLACE VIEW code_units_view AS
    SELECT u.id, u.version_id, r.name AS repo, v.tag AS version, u.path AS file, u.kind, u.name,
           u.start_line, u.end_line, u.text, u.symbol_id
    FROM code_units u
    JOIN versions v     ON v.id = u.version_id
    JOIN repositories r ON r.id = v.repository_id;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'harvest_graph_reader') THEN
        GRANT SELECT ON code_units_view TO harvest_graph_reader;
    END IF;
EXCEPTION WHEN insufficient_privilege THEN
    RAISE NOTICE 'could not grant on code_units_view';
END $$;

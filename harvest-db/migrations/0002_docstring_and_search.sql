
ALTER TABLE symbols ADD COLUMN IF NOT EXISTS docstring text;

CREATE INDEX IF NOT EXISTS symbols_signature_trgm ON symbols USING gin (lower(signature) gin_trgm_ops);
CREATE INDEX IF NOT EXISTS symbols_docstring_trgm  ON symbols USING gin (lower(docstring) gin_trgm_ops);

CREATE OR REPLACE VIEW code_symbols AS
    SELECT s.id, s.file_id, s.version_id, r.name AS repo, v.tag AS version, f.path AS file,
           s.label, s.name, s.kind, s.signature, s.start_line, s.end_line, s.source,
           s.impl_type, s.bases, s.traits, s.embeds, s.uses, s.docstring
    FROM symbols s
    JOIN files f        ON f.id = s.file_id
    JOIN versions v     ON v.id = s.version_id
    JOIN repositories r ON r.id = v.repository_id;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'harvest_graph_reader') THEN
        GRANT SELECT ON code_symbols TO harvest_graph_reader;
    END IF;
EXCEPTION WHEN insufficient_privilege THEN
    RAISE NOTICE 'could not grant on code_symbols';
END $$;

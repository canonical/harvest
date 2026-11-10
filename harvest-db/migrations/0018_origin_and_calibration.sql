ALTER TABLE files ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'source';
ALTER TABLE files DROP CONSTRAINT IF EXISTS files_origin_check;
ALTER TABLE files ADD CONSTRAINT files_origin_check
    CHECK (origin IN ('source', 'test', 'vendored', 'generated'));
ALTER TABLE symbols ADD COLUMN IF NOT EXISTS is_test boolean NOT NULL DEFAULT false;

CREATE OR REPLACE VIEW code_files AS
    SELECT f.id, f.version_id, r.name AS repo, v.tag AS version, f.path, f.language, f.origin
    FROM files f
    JOIN versions v     ON v.id = f.version_id
    JOIN repositories r ON r.id = v.repository_id;

CREATE OR REPLACE VIEW code_symbols AS
    SELECT s.id, s.file_id, s.version_id, r.name AS repo, v.tag AS version, f.path AS file,
           s.label, s.name, s.kind, s.signature, s.start_line, s.end_line, s.source,
           s.impl_type, s.bases, s.traits, s.embeds, s.uses, s.docstring, s.decorators,
           s.is_test, f.origin
    FROM symbols s
    JOIN files f        ON f.id = s.file_id
    JOIN versions v     ON v.id = s.version_id
    JOIN repositories r ON r.id = v.repository_id;

CREATE TABLE IF NOT EXISTS search_calibrations (
    version_id  bigint PRIMARY KEY REFERENCES versions ON DELETE CASCADE,
    reranker    jsonb       NOT NULL,
    gate        jsonb       NOT NULL,
    queries     integer     NOT NULL,
    metrics     jsonb       NOT NULL DEFAULT '{}',
    created_at  timestamptz NOT NULL DEFAULT now()
);

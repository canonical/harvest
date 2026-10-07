ALTER TABLE symbols ADD COLUMN IF NOT EXISTS decorators text[] NOT NULL DEFAULT '{}';

CREATE OR REPLACE VIEW code_symbols AS
    SELECT s.id, s.file_id, s.version_id, r.name AS repo, v.tag AS version, f.path AS file,
           s.label, s.name, s.kind, s.signature, s.start_line, s.end_line, s.source,
           s.impl_type, s.bases, s.traits, s.embeds, s.uses, s.docstring, s.decorators
    FROM symbols s
    JOIN files f        ON f.id = s.file_id
    JOIN versions v     ON v.id = s.version_id
    JOIN repositories r ON r.id = v.repository_id;

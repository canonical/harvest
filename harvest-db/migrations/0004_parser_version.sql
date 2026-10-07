ALTER TABLE versions ADD COLUMN IF NOT EXISTS parser_version integer NOT NULL DEFAULT 0;

CREATE OR REPLACE VIEW code_versions AS
    SELECT v.id, v.repository_id, r.name AS repo, v.tag, v.commit_sha, v.timestamp, v.ingested,
           v.parser_version
    FROM versions v JOIN repositories r ON r.id = v.repository_id;

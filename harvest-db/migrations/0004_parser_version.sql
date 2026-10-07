-- Which harvester parser version produced each ingested version. A version ingested by an
-- older parser is missing whatever later parser fixes extract, so the harvester re-ingests it.
ALTER TABLE versions ADD COLUMN IF NOT EXISTS parser_version integer NOT NULL DEFAULT 0;

CREATE OR REPLACE VIEW code_versions AS
    SELECT v.id, v.repository_id, r.name AS repo, v.tag, v.commit_sha, v.timestamp, v.ingested,
           v.parser_version
    FROM versions v JOIN repositories r ON r.id = v.repository_id;

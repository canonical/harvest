CREATE TABLE code_previews (
    id            text        PRIMARY KEY,
    session_id    text        NOT NULL UNIQUE REFERENCES code_sessions ON DELETE CASCADE,
    kind          text        NOT NULL CHECK (kind IN ('web', 'api', 'cli')),
    port          integer     CHECK (port BETWEEN 1 AND 65535),
    start_command text,
    audience      text        NOT NULL DEFAULT 'owner' CHECK (audience IN ('owner', 'repository', 'link')),
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE code_preview_viewers (
    preview_id   text        NOT NULL REFERENCES code_previews ON DELETE CASCADE,
    viewer       text        NOT NULL,
    container    text        NOT NULL REFERENCES code_containers (name) ON DELETE CASCADE,
    error        text,
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (preview_id, viewer)
);

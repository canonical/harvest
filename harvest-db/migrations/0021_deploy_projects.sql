ALTER TABLE projects RENAME TO deploy_projects;

CREATE TABLE deploy_project_members (
    project_id text        NOT NULL REFERENCES deploy_projects ON DELETE CASCADE,
    user_id    text        NOT NULL REFERENCES users ON DELETE CASCADE,
    role       text        NOT NULL CHECK (role IN ('owner', 'editor', 'viewer')),
    added_by   text,
    added_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, user_id)
);
CREATE INDEX deploy_project_members_user ON deploy_project_members (user_id);

INSERT INTO deploy_project_members (project_id, user_id, role)
SELECT p.id, ug.user_id, 'editor'
FROM deploy_projects p
JOIN user_groups ug ON ug.group_id = p.group_id
ON CONFLICT (project_id, user_id) DO NOTHING;

INSERT INTO deploy_project_members (project_id, user_id, role)
SELECT p.id, p.created_by, 'owner'
FROM deploy_projects p
JOIN users u ON u.id = p.created_by
ON CONFLICT (project_id, user_id) DO UPDATE SET role = 'owner';

ALTER TABLE deploy_projects DROP COLUMN group_id;
DROP TABLE user_groups;
DROP TABLE groups;

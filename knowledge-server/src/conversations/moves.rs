use serde_json::{json, Value};

use harvest_db::Db;

pub const CODE_TOOL_NAMES: &[&str] = &[
    "ask_user",
    "list_repositories",
    "search_code",
    "search_symbols",
    "read_sources",
    "get_symbol_source",
    "get_file_symbols",
    "find_callers",
    "find_callees",
    "get_imports",
    "compare_symbol_across_versions",
    "get_capability_matrix",
    "get_evidence_pack",
    "find_subclasses",
    "run_sql",
];

const TOOL_CALLS: &str = "SELECT m.conv_id,
           jsonb_path_query(m.payload, '$.chain[*] ? (@.type == \"tool_call\").name') #>> '{}' AS name
    FROM conversation_messages m";

pub async fn code_only_project_conversations(db: &Db) -> anyhow::Result<Vec<Value>> {
    db.query(
        &format!(
            "WITH tools AS ({TOOL_CALLS})
             SELECT c.id, c.title, c.updated_at, p.id AS project_id, p.name AS project_name,
                    u.id AS author_id, u.email AS author_email
             FROM conversations c
             JOIN deploy_projects p ON p.id = c.project_id
             JOIN users u ON u.id = c.created_by
             WHERE EXISTS (SELECT 1 FROM tools t WHERE t.conv_id = c.id)
               AND NOT EXISTS (SELECT 1 FROM tools t
                               WHERE t.conv_id = c.id AND t.name <> ALL($code_tools::text[]))
             ORDER BY p.name, c.updated_at DESC"
        ),
        json!({ "code_tools": CODE_TOOL_NAMES }),
    )
    .await
}

pub async fn move_to_authors(db: &Db, conversation_ids: &[String]) -> anyhow::Result<usize> {
    let moved = db
        .query(
            "UPDATE conversations c SET user_id = c.created_by, project_id = NULL
             FROM users u
             WHERE u.id = c.created_by AND c.id = ANY($ids::text[]) AND c.project_id IS NOT NULL
             RETURNING c.id",
            json!({ "ids": conversation_ids }),
        )
        .await?;
    Ok(moved.len())
}

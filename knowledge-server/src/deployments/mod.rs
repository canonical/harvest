pub mod handlers;

use serde_json::{json, Value};

use crate::machines::TerraformAction;
use crate::neo4j::Neo4jClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfraState {
    None,
    Up,
    Broken,
    Destroyed,
    DestroyFailed,
}

impl InfraState {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none"           => Some(Self::None),
            "up"              => Some(Self::Up),
            "broken"          => Some(Self::Broken),
            "destroyed"       => Some(Self::Destroyed),
            "destroy_failed"  => Some(Self::DestroyFailed),
            _                 => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None          => "none",
            Self::Up             => "up",
            Self::Broken         => "broken",
            Self::Destroyed      => "destroyed",
            Self::DestroyFailed  => "destroy_failed",
        }
    }
}

pub fn next_infra_state(action: TerraformAction, success: bool) -> Option<InfraState> {
    match (action, success) {
        (TerraformAction::Plan, _)        => None,
        (TerraformAction::Apply, true)    => Some(InfraState::Up),
        (TerraformAction::Apply, false)   => Some(InfraState::Broken),
        (TerraformAction::Destroy, true)  => Some(InfraState::Destroyed),
        (TerraformAction::Destroy, false) => Some(InfraState::DestroyFailed),
    }
}

pub fn needs_destroy_before_apply(state: InfraState) -> bool {
    matches!(state, InfraState::Broken | InfraState::DestroyFailed)
}

pub fn should_trigger_triage(action: TerraformAction, success: bool) -> bool {
    matches!(action, TerraformAction::Apply | TerraformAction::Destroy) && !success
}

/// A deployment can end up `broken`/`destroy_failed` with no `last_applied_content` when its
/// very first apply fails — there's nothing terraform ever actually created, so there's nothing
/// to destroy either. Without this, the deployment would be permanently stuck: `broken` disables
/// Deploy and every Redeploy/Destroy attempt fails the same "nothing was ever applied" way. Reset
/// it back to `none` so the user can just deploy again.
pub async fn reset_infra_state_to_none(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         SET d.infra_state = 'none', d.updated_at = $now",
        json!({ "pid": project_id, "did": deployment_id, "now": now }),
    ).await?;
    Ok(())
}

const RUN_STDOUT_PREVIEW_CHARS: usize = 2000;
const RUN_STDERR_PREVIEW_CHARS: usize = 2000;

fn preview(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[allow(clippy::too_many_arguments)]
pub async fn record_run_and_update_state(
    neo4j:           &Neo4jClient,
    project_id:      &str,
    artifact_id:     &str,
    action:          TerraformAction,
    exit_code:       Option<i32>,
    stdout:          &str,
    stderr:          &str,
    applied_content: Option<&str>,
    initiated_by:    &str,
    reasoning:       Option<&str>,
) -> anyhow::Result<Option<String>> {
    let linked = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)-[:HAS_TERRAFORM_BUNDLE]->(:Artifact {id: $aid})
         RETURN d.id AS id",
        json!({ "pid": project_id, "aid": artifact_id }),
    ).await?;
    let Some(deployment_id) = linked.into_iter().next().and_then(|r| r["id"].as_str().map(str::to_string)) else {
        return Ok(None);
    };

    let success       = exit_code == Some(0);
    let Some(new_state) = next_infra_state(action, success) else {
        return Ok(None);
    };

    let now = chrono::Utc::now().to_rfc3339();
    let rid = uuid::Uuid::new_v4().to_string();
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         CREATE (r:DeploymentRun {
             id: $rid, action: $action, status: $status, exit_code: $exit_code,
             stdout_preview: $stdout_preview, stderr_preview: $stderr_preview,
             initiated_by: $initiated_by, reasoning: $reasoning, created_at: $now
         })
         CREATE (d)-[:HAS_RUN]->(r)
         SET d.infra_state = $new_state, d.updated_at = $now",
        json!({
            "pid": project_id, "did": deployment_id, "rid": rid,
            "action": action.as_str(), "status": if success { "success" } else { "failed" },
            "exit_code": exit_code, "stdout_preview": preview(stdout, RUN_STDOUT_PREVIEW_CHARS),
            "stderr_preview": preview(stderr, RUN_STDERR_PREVIEW_CHARS),
            "initiated_by": initiated_by, "reasoning": reasoning, "now": now,
            "new_state": new_state.as_str(),
        }),
    ).await?;

    if matches!(action, TerraformAction::Apply) && success {
        if let Some(content) = applied_content {
            neo4j.query_read(
                "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
                 SET d.last_applied_content = $content, d.last_applied_artifact_id = $aid, d.last_applied_at = $now",
                json!({ "pid": project_id, "did": deployment_id, "content": content, "aid": artifact_id, "now": now }),
            ).await?;
        }
    }

    Ok(Some(new_state.as_str().to_string()))
}

pub async fn last_applied_bundle_for_artifact(
    neo4j:       &Neo4jClient,
    project_id:  &str,
    artifact_id: &str,
) -> anyhow::Result<Option<String>> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)-[:HAS_TERRAFORM_BUNDLE]->(:Artifact {id: $aid})
         RETURN d.last_applied_content AS last_applied_content",
        json!({ "pid": project_id, "aid": artifact_id }),
    ).await?;
    Ok(rows.into_iter().next().and_then(|r| r["last_applied_content"].as_str().map(str::to_string)))
}

pub async fn resolve_run_content(
    neo4j:        &Neo4jClient,
    project_id:   &str,
    artifact_id:  &str,
    action:       TerraformAction,
    live_content: &str,
) -> anyhow::Result<String> {
    if action == TerraformAction::Destroy {
        if let Some(applied) = last_applied_bundle_for_artifact(neo4j, project_id, artifact_id).await? {
            return Ok(applied);
        }
    }
    Ok(live_content.to_string())
}

#[derive(Debug, Clone)]
pub struct FailedRun {
    pub id:              String,
    pub action:         String,
    pub exit_code:       Option<i64>,
    pub stdout_preview:  String,
    pub stderr_preview:  String,
}

pub async fn latest_failed_run(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
) -> anyhow::Result<Option<FailedRun>> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment {id: $did})-[:HAS_RUN]->(r:DeploymentRun {status: 'failed'})
         RETURN r.id AS id, r.action AS action, r.exit_code AS exit_code,
                r.stdout_preview AS stdout_preview, r.stderr_preview AS stderr_preview
         ORDER BY r.created_at DESC LIMIT 1",
        json!({ "pid": project_id, "did": deployment_id }),
    ).await?;
    Ok(rows.into_iter().next().map(|r| FailedRun {
        id:            r["id"].as_str().unwrap_or_default().to_string(),
        action:        r["action"].as_str().unwrap_or_default().to_string(),
        exit_code:     r["exit_code"].as_i64(),
        stdout_preview: r["stdout_preview"].as_str().unwrap_or_default().to_string(),
        stderr_preview: r["stderr_preview"].as_str().unwrap_or_default().to_string(),
    }))
}

pub struct PriorDeploymentSummary {
    pub name:                     String,
    pub environment_description: String,
    pub infra_state:              String,
}

pub struct DeploymentContext {
    pub deployment_id:            String,
    pub deployment_name:          String,
    pub environment_description: String,
    pub infra_state:              String,
    pub product_template_name:    Option<String>,
    pub product_template_content: Option<String>,
    pub prior_deployments:        Vec<PriorDeploymentSummary>,
}

const MAX_PRIOR_DEPLOYMENTS: i64 = 5;

pub async fn load_deployment_context(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
) -> anyhow::Result<Option<DeploymentContext>> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         OPTIONAL MATCH (d)-[:USES_TEMPLATE]->(t:ProductTemplate)
         RETURN d.name AS name, d.environment_description AS environment_description,
                d.infra_state AS infra_state,
                t.id AS template_id, t.name AS template_name, t.content AS template_content",
        json!({ "pid": project_id, "did": deployment_id }),
    ).await?;
    let Some(row) = rows.into_iter().next() else { return Ok(None) };

    let prior_deployments = match opt_str(&row, "template_id") {
        Some(template_id) => {
            let prior_rows = neo4j.query_read(
                "MATCH (:ProductTemplate {id: $tid})<-[:USES_TEMPLATE]-(other:Deployment)
                 WHERE other.id <> $did
                 RETURN other.name AS name, other.environment_description AS environment_description,
                        other.infra_state AS infra_state
                 ORDER BY other.updated_at DESC LIMIT $limit",
                json!({ "tid": template_id, "did": deployment_id, "limit": MAX_PRIOR_DEPLOYMENTS }),
            ).await?;
            prior_rows.iter().map(|r| PriorDeploymentSummary {
                name:                     r["name"].as_str().unwrap_or_default().to_string(),
                environment_description: r["environment_description"].as_str().unwrap_or_default().to_string(),
                infra_state:              r["infra_state"].as_str().unwrap_or_default().to_string(),
            }).collect()
        }
        None => Vec::new(),
    };

    Ok(Some(DeploymentContext {
        deployment_id:            deployment_id.to_string(),
        deployment_name:          row["name"].as_str().unwrap_or_default().to_string(),
        environment_description: row["environment_description"].as_str().unwrap_or_default().to_string(),
        infra_state:              row["infra_state"].as_str().unwrap_or("none").to_string(),
        product_template_name:    opt_str(&row, "template_name"),
        product_template_content: opt_str(&row, "template_content"),
        prior_deployments,
    }))
}

fn balanced_json_span(text: &str, open: char, close: char) -> Option<&str> {
    let start = text.find(open)?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in text[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
        } else if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                let end = start + i + c.len_utf8();
                return Some(&text[start..end]);
            }
        }
    }
    None
}

fn fallback_json_span(text: &str) -> Option<&str> {
    let obj = balanced_json_span(text, '{', '}');
    let arr = balanced_json_span(text, '[', ']');
    match (obj, arr) {
        (Some(o), Some(a)) => {
            let obj_start = text.find('{').unwrap_or(usize::MAX);
            let arr_start = text.find('[').unwrap_or(usize::MAX);
            Some(if obj_start <= arr_start { o } else { a })
        }
        (Some(o), None) => Some(o),
        (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

pub fn extract_json_block(text: &str) -> Option<Value> {
    if let Some(fence_start) = text.find("```json") {
        let after_fence = &text[fence_start + "```json".len()..];
        if let Some(fence_end) = after_fence.find("```") {
            let candidate = after_fence[..fence_end].trim();
            if let Ok(value) = serde_json::from_str(candidate) {
                return Some(value);
            }
        }
    }
    fallback_json_span(text).and_then(|span| serde_json::from_str(span).ok())
}

fn opt_str(row: &Value, key: &str) -> Option<String> {
    row[key].as_str().map(str::to_string)
}

fn artifact_ref(row: &Value, id_key: &str, title_key: &str, kind_key: Option<&str>) -> Value {
    let Some(id) = opt_str(row, id_key) else { return Value::Null };
    let mut obj = json!({ "id": id, "title": opt_str(row, title_key) });
    if let Some(k) = kind_key {
        obj["kind"] = json!(opt_str(row, k));
    }
    obj
}

pub fn shape_deployment(row: &Value) -> Value {
    let template = match opt_str(row, "template_id") {
        Some(id) => json!({ "id": id, "name": opt_str(row, "template_name") }),
        None => Value::Null,
    };
    json!({
        "id":                       row["id"],
        "name":                     row["name"],
        "environment_description": row["environment_description"],
        "infra_state":              row["infra_state"],
        "last_applied_artifact_id": row["last_applied_artifact_id"],
        "last_applied_at":          row["last_applied_at"],
        "created_by":               row["created_by"],
        "created_at":               row["created_at"],
        "updated_at":               row["updated_at"],
        "template":                 template,
        "design_doc":               artifact_ref(row, "design_doc_id", "design_doc_title", None),
        "terraform_bundle":         artifact_ref(row, "terraform_bundle_id", "terraform_bundle_title", Some("terraform_bundle_kind")),
        "guide":                    artifact_ref(row, "guide_id", "guide_title", None),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_block_parses_fenced_json_object() {
        let text = "Here you go:\n```json\n{\"a\": 1}\n```\nDone.";
        assert_eq!(extract_json_block(text), Some(json!({ "a": 1 })));
    }

    #[test]
    fn extract_json_block_parses_fenced_json_array() {
        let text = "```json\n[1, 2, 3]\n```";
        assert_eq!(extract_json_block(text), Some(json!([1, 2, 3])));
    }

    #[test]
    fn extract_json_block_falls_back_to_balanced_object_without_fence() {
        let text = "Sure, here you go: {\"a\": 1} thanks";
        assert_eq!(extract_json_block(text), Some(json!({ "a": 1 })));
    }

    #[test]
    fn extract_json_block_falls_back_to_balanced_array_without_fence() {
        let text = "The list is [1, 2, 3] as requested.";
        assert_eq!(extract_json_block(text), Some(json!([1, 2, 3])));
    }

    #[test]
    fn extract_json_block_handles_nested_braces() {
        let text = "Explanation first.\n\n{\"a\": {\"b\": 1}, \"c\": [1, 2]}\n\nTrailing text.";
        assert_eq!(extract_json_block(text), Some(json!({ "a": { "b": 1 }, "c": [1, 2] })));
    }

    #[test]
    fn extract_json_block_ignores_braces_inside_string_literals() {
        let text = "{\"text\": \"a { b } c\"}";
        assert_eq!(extract_json_block(text), Some(json!({ "text": "a { b } c" })));
    }

    #[test]
    fn extract_json_block_returns_none_for_no_json() {
        let text = "There is no JSON in this response at all.";
        assert_eq!(extract_json_block(text), None);
    }

    #[test]
    fn extract_json_block_returns_none_for_malformed_json() {
        let text = "```json\n{not valid json\n```";
        assert_eq!(extract_json_block(text), None);
    }

    #[test]
    fn extract_json_block_prefers_fenced_block_over_stray_braces() {
        let text = "note: {not json}\n```json\n{\"a\": 1}\n```";
        assert_eq!(extract_json_block(text), Some(json!({ "a": 1 })));
    }

    #[test]
    fn extract_json_block_handles_multibyte_characters_around_and_inside_the_json() {
        let text = "Here's the plan — it's straightforward: {\"café\": \"düsseldorf — ready\"} — done.";
        assert_eq!(extract_json_block(text), Some(json!({ "café": "düsseldorf — ready" })));
    }

    #[test]
    fn next_infra_state_plan_never_changes_state() {
        assert_eq!(next_infra_state(TerraformAction::Plan, true), None);
        assert_eq!(next_infra_state(TerraformAction::Plan, false), None);
    }

    #[test]
    fn next_infra_state_successful_apply_is_up() {
        assert_eq!(next_infra_state(TerraformAction::Apply, true), Some(InfraState::Up));
    }

    #[test]
    fn next_infra_state_failed_apply_is_broken() {
        assert_eq!(next_infra_state(TerraformAction::Apply, false), Some(InfraState::Broken));
    }

    #[test]
    fn next_infra_state_successful_destroy_is_destroyed() {
        assert_eq!(next_infra_state(TerraformAction::Destroy, true), Some(InfraState::Destroyed));
    }

    #[test]
    fn next_infra_state_failed_destroy_is_destroy_failed() {
        assert_eq!(next_infra_state(TerraformAction::Destroy, false), Some(InfraState::DestroyFailed));
    }

    #[test]
    fn should_trigger_triage_true_for_failed_apply() {
        assert!(should_trigger_triage(TerraformAction::Apply, false));
    }

    #[test]
    fn should_trigger_triage_false_for_successful_apply() {
        assert!(!should_trigger_triage(TerraformAction::Apply, true));
    }

    #[test]
    fn should_trigger_triage_true_for_failed_destroy() {
        assert!(should_trigger_triage(TerraformAction::Destroy, false));
    }

    #[test]
    fn should_trigger_triage_false_for_successful_destroy() {
        assert!(!should_trigger_triage(TerraformAction::Destroy, true));
    }

    #[test]
    fn should_trigger_triage_false_for_plan_regardless_of_success() {
        assert!(!should_trigger_triage(TerraformAction::Plan, true));
        assert!(!should_trigger_triage(TerraformAction::Plan, false));
    }

    #[test]
    fn needs_destroy_before_apply_true_only_for_broken_or_destroy_failed() {
        assert!(!needs_destroy_before_apply(InfraState::None));
        assert!(!needs_destroy_before_apply(InfraState::Up));
        assert!(needs_destroy_before_apply(InfraState::Broken));
        assert!(!needs_destroy_before_apply(InfraState::Destroyed));
        assert!(needs_destroy_before_apply(InfraState::DestroyFailed));
    }

    #[test]
    fn infra_state_parse_and_as_str_round_trip() {
        for state in [InfraState::None, InfraState::Up, InfraState::Broken, InfraState::Destroyed, InfraState::DestroyFailed] {
            assert_eq!(InfraState::parse(state.as_str()), Some(state));
        }
    }

    #[test]
    fn infra_state_parse_rejects_unknown_string() {
        assert_eq!(InfraState::parse("nonsense"), None);
    }

    fn base_row() -> Value {
        json!({
            "id": "d1", "name": "Acme rollout", "environment_description": "on-prem, air-gapped",
            "infra_state": "none", "last_applied_artifact_id": Value::Null, "last_applied_at": Value::Null,
            "created_by": "u1", "created_at": "t1", "updated_at": "t1",
            "template_id": Value::Null, "template_name": Value::Null,
            "design_doc_id": Value::Null, "design_doc_title": Value::Null,
            "terraform_bundle_id": Value::Null, "terraform_bundle_title": Value::Null, "terraform_bundle_kind": Value::Null,
            "guide_id": Value::Null, "guide_title": Value::Null,
        })
    }

    #[test]
    fn shape_deployment_omits_template_when_absent() {
        let shaped = shape_deployment(&base_row());
        assert_eq!(shaped["template"], Value::Null);
    }

    #[test]
    fn shape_deployment_includes_template_when_present() {
        let mut row = base_row();
        row["template_id"] = json!("t1");
        row["template_name"] = json!("Acme Gateway v3");
        let shaped = shape_deployment(&row);
        assert_eq!(shaped["template"]["id"], "t1");
        assert_eq!(shaped["template"]["name"], "Acme Gateway v3");
    }

    #[test]
    fn shape_deployment_omits_artifacts_when_absent() {
        let shaped = shape_deployment(&base_row());
        assert_eq!(shaped["design_doc"], Value::Null);
        assert_eq!(shaped["terraform_bundle"], Value::Null);
        assert_eq!(shaped["guide"], Value::Null);
    }

    #[test]
    fn shape_deployment_includes_terraform_bundle_with_kind_when_present() {
        let mut row = base_row();
        row["terraform_bundle_id"] = json!("a1");
        row["terraform_bundle_title"] = json!("Infra bundle");
        row["terraform_bundle_kind"] = json!("terraform");
        let shaped = shape_deployment(&row);
        assert_eq!(shaped["terraform_bundle"]["id"], "a1");
        assert_eq!(shaped["terraform_bundle"]["title"], "Infra bundle");
        assert_eq!(shaped["terraform_bundle"]["kind"], "terraform");
    }

    #[test]
    fn shape_deployment_includes_design_doc_without_kind_field() {
        let mut row = base_row();
        row["design_doc_id"] = json!("a2");
        row["design_doc_title"] = json!("Design");
        let shaped = shape_deployment(&row);
        assert_eq!(shaped["design_doc"]["id"], "a2");
        assert!(shaped["design_doc"].get("kind").is_none());
    }

    #[test]
    fn shape_deployment_passes_through_scalar_fields() {
        let shaped = shape_deployment(&base_row());
        assert_eq!(shaped["id"], "d1");
        assert_eq!(shaped["name"], "Acme rollout");
        assert_eq!(shaped["infra_state"], "none");
    }

}

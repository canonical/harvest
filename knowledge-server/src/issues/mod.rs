pub mod handlers;

use serde_json::{json, Value};

use crate::deployments::FailedRun;
use crate::neo4j::Neo4jClient;

pub fn map_issue_status_to_cr(s: &str) -> &'static str {
    match s {
        "untriaged"   => "open",
        "in_progress" => "in_review",
        "fixed"       => "applied",
        "rejected"    => "discarded",
        _             => "open",
    }
}

pub fn map_proposal_status_to_cr(s: &str) -> &'static str {
    match s {
        "pending"   => "open",
        "approved"  => "applied",
        "discarded" => "discarded",
        _           => "open",
    }
}

pub fn cr_status_to_issue_status(s: &str) -> Option<&'static str> {
    match s {
        "open"      => Some("untriaged"),
        "in_review" => Some("in_progress"),
        "applied"   => Some("fixed"),
        "discarded" => Some("rejected"),
        _           => None,
    }
}

pub fn cr_status_to_proposal_status(s: &str) -> Option<&'static str> {
    match s {
        "open"      => Some("pending"),
        "applied"   => Some("approved"),
        "discarded" => Some("discarded"),
        _           => None,
    }
}

pub async fn append_proposal_comment(
    neo4j:          &Neo4jClient,
    project_id:     &str,
    proposal_id:    &str,
    author_type:    &str,
    author_name:    &str,
    body:           &str,
) -> anyhow::Result<()> {
    let exists = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_PROPOSAL]->(p:Proposal {id: $prid})
         RETURN p.comments AS comments",
        json!({ "pid": project_id, "prid": proposal_id }),
    ).await?;
    let Some(row) = exists.into_iter().next() else { return Ok(()) };
    let mut comments: Vec<Value> = row["comments"].as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let now = chrono::Utc::now().to_rfc3339();
    comments.push(json!({
        "id":          uuid::Uuid::new_v4().to_string(),
        "author_type": author_type,
        "author_name": author_name,
        "body":        body,
        "created_at":  now,
    }));
    let comments_json = serde_json::to_string(&comments)?;
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_PROPOSAL]->(p:Proposal {id: $prid})
         SET p.comments = $comments",
        json!({ "pid": project_id, "prid": proposal_id, "comments": comments_json }),
    ).await?;
    Ok(())
}

pub async fn list_change_requests_for_project(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    status:        Option<&str>,
    deployment_id: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    let cypher = "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)-[:HAS_ISSUE]->(i:Issue)
                  WHERE ($status IS NULL OR
                        CASE i.status
                          WHEN 'untriaged'   THEN 'open'
                          WHEN 'in_progress' THEN 'in_review'
                          WHEN 'fixed'       THEN 'applied'
                          WHEN 'rejected'    THEN 'discarded'
                        END = $status)
                    AND ($did IS NULL OR d.id = $did)
                  RETURN i.id AS id, i.title AS title, i.status AS raw_status, 'issue' AS kind,
                         i.created_at AS created_at, i.updated_at AS updated_at,
                         d.id AS deployment_id, d.name AS deployment_name

                  UNION

                  MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)-[:HAS_PROPOSAL]->(p:Proposal)
                  OPTIONAL MATCH (p)-[:TARGETS]->(a:Artifact)
                  WHERE ($status IS NULL OR
                        CASE p.status
                          WHEN 'pending'   THEN 'open'
                          WHEN 'approved'  THEN 'applied'
                          WHEN 'discarded' THEN 'discarded'
                        END = $status)
                    AND ($did IS NULL OR d.id = $did)
                  RETURN p.id AS id, p.source AS title, p.status AS raw_status, 'proposal' AS kind,
                         p.created_at AS created_at, p.created_at AS updated_at,
                         d.id AS deployment_id, d.name AS deployment_name";
    let rows = neo4j.query_read(
        cypher,
        json!({ "pid": project_id, "status": status, "did": deployment_id }),
    ).await?;
    let shaped: Vec<Value> = rows.iter().map(|row| {
        let raw = row["raw_status"].as_str().unwrap_or("");
        let kind = row["kind"].as_str().unwrap_or("issue");
        let unified = if kind == "proposal" {
            map_proposal_status_to_cr(raw)
        } else {
            map_issue_status_to_cr(raw)
        };
        json!({
            "id":             row["id"],
            "title":          row["title"],
            "status":         unified,
            "kind":           kind,
            "created_at":     row["created_at"],
            "updated_at":     row["updated_at"],
            "deployment":     json!({
                "id":   row["deployment_id"],
                "name": row["deployment_name"],
            }),
        })
    }).collect();
    Ok(shaped)
}

pub async fn get_change_request_detail(
    neo4j:      &Neo4jClient,
    project_id: &str,
    cr_id:      &str,
) -> anyhow::Result<Option<Value>> {
    let issue_rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $cr_id})
         RETURN i.id AS id, i.title AS title, i.description AS description, i.status AS status,
                i.fingerprint AS fingerprint, i.proposed_solution_summary AS proposed_solution_summary,
                i.proposed_files AS proposed_files, i.chat_messages AS chat_messages, i.comments AS comments,
                i.created_by AS created_by, i.created_at AS created_at, i.updated_at AS updated_at,
                d.id AS deployment_id, d.name AS deployment_name, d.infra_state AS deployment_infra_state",
        json!({ "pid": project_id, "cr_id": cr_id }),
    ).await?;
    if let Some(row) = issue_rows.into_iter().next() {
        let mut shaped = shape_issue(&row);
        shaped["kind"] = json!("issue");
        shaped["status"] = json!(map_issue_status_to_cr(row["status"].as_str().unwrap_or("")));
        return Ok(Some(shaped));
    }

    let proposal_rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)-[:HAS_PROPOSAL]->(p:Proposal {id: $cr_id})
         OPTIONAL MATCH (p)-[:TARGETS]->(a:Artifact)
         RETURN p.id AS id, p.source AS source, p.explanation AS explanation,
                p.current_content AS current_content, p.proposed_content AS proposed_content,
                p.status AS status, p.created_at AS created_at, p.comments AS comments,
                a.id AS target_artifact_id, a.kind AS target_artifact_kind, a.title AS target_artifact_title,
                d.id AS deployment_id, d.name AS deployment_name, d.infra_state AS deployment_infra_state",
        json!({ "pid": project_id, "cr_id": cr_id }),
    ).await?;
    if let Some(row) = proposal_rows.into_iter().next() {
        return Ok(Some(json!({
            "id":                   row["id"],
            "kind":                 "proposal",
            "status":               map_proposal_status_to_cr(row["status"].as_str().unwrap_or("")),
            "title":                row["source"],
            "source":               row["source"],
            "explanation":          row["explanation"],
            "current_content":      row["current_content"],
            "proposed_content":     row["proposed_content"],
            "target_artifact_id":   row["target_artifact_id"],
            "target_artifact_kind": row["target_artifact_kind"],
            "target_artifact_title":row["target_artifact_title"],
            "created_at":           row["created_at"],
            "comments":             parse_json_array(&row, "comments"),
            "deployment":           json!({
                "id":          row["deployment_id"],
                "name":        row["deployment_name"],
                "infra_state": row["deployment_infra_state"],
            }),
        })));
    }
    Ok(None)
}

pub async fn change_request_kind(
    neo4j:      &Neo4jClient,
    project_id: &str,
    cr_id:      &str,
) -> anyhow::Result<Option<&'static str>> {
    let issue_rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $cr_id})
         RETURN 1",
        json!({ "pid": project_id, "cr_id": cr_id }),
    ).await?;
    if !issue_rows.is_empty() {
        return Ok(Some("issue"));
    }
    let proposal_rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_PROPOSAL]->(p:Proposal {id: $cr_id})
         RETURN 1",
        json!({ "pid": project_id, "cr_id": cr_id }),
    ).await?;
    if !proposal_rows.is_empty() {
        return Ok(Some("proposal"));
    }
    Ok(None)
}

pub async fn discard_issue_cr(
    neo4j:      &Neo4jClient,
    project_id: &str,
    issue_id:   &str,
) -> anyhow::Result<bool> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         WHERE i.status IN ['untriaged', 'in_progress']
         SET i.status = 'rejected', i.updated_at = $now
         RETURN 1",
        json!({ "pid": project_id, "iid": issue_id, "now": chrono::Utc::now().to_rfc3339() }),
    ).await?;
    Ok(!rows.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueStatus {
    Untriaged,
    InProgress,
    Fixed,
    Rejected,
}

impl IssueStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "untriaged"   => Some(Self::Untriaged),
            "in_progress" => Some(Self::InProgress),
            "fixed"       => Some(Self::Fixed),
            "rejected"    => Some(Self::Rejected),
            _             => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Untriaged  => "untriaged",
            Self::InProgress => "in_progress",
            Self::Fixed      => "fixed",
            Self::Rejected   => "rejected",
        }
    }
}

pub fn validate_transition(current: IssueStatus, requested: IssueStatus) -> Result<(), String> {
    use IssueStatus::*;
    match (current, requested) {
        (Untriaged, InProgress) => Ok(()),
        (InProgress, Fixed)     => Ok(()),
        (InProgress, Rejected)  => Ok(()),
        _ => Err(format!(
            "cannot move an issue from '{}' to '{}'",
            current.as_str(), requested.as_str()
        )),
    }
}

pub fn is_regression_reopen(current: IssueStatus) -> bool {
    matches!(current, IssueStatus::Fixed | IssueStatus::Rejected)
}

const RUN_SNIPPET_CHARS: usize = 400;

fn run_snippet(run: &FailedRun) -> String {
    let source = if !run.stderr_preview.trim().is_empty() {
        &run.stderr_preview
    } else {
        &run.stdout_preview
    };
    source.chars().take(RUN_SNIPPET_CHARS).collect()
}

pub fn issue_created_comment(run: &FailedRun) -> String {
    let exit = run.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".to_string());
    format!(
        "Created from a failed **{}** run (exit {exit}, run `{}`):\n\n```\n{}\n```",
        run.action, run.id, run_snippet(run),
    )
}

pub fn run_linked_comment(run: &FailedRun) -> String {
    format!(
        "Linked to a new failed **{}** run (`{}`) with the same root cause:\n\n```\n{}\n```",
        run.action, run.id, run_snippet(run),
    )
}

pub fn regression_reopened_comment(run: &FailedRun, previous_status: IssueStatus) -> String {
    format!(
        "Reopened: this issue was marked **{}**, but the same failure resurfaced in run `{}` \
         (**{}**):\n\n```\n{}\n```",
        previous_status.as_str(), run.id, run.action, run_snippet(run),
    )
}

pub fn status_changed_comment(from: IssueStatus, to: IssueStatus, changed_by: &str) -> String {
    format!("{changed_by} moved this issue from **{}** to **{}**.", from.as_str(), to.as_str())
}

pub fn solution_proposed_comment(summary: &str, file_count: usize) -> String {
    format!("Proposed a fix touching {file_count} file(s): {summary}")
}

pub fn solution_applied_comment(summary: &str) -> String {
    format!("Applied the proposed fix and triggered a redeploy: {summary}")
}

fn parse_json_array(row: &Value, key: &str) -> Value {
    row[key].as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or_else(|| json!([]))
}

pub fn shape_issue_summary(row: &Value) -> Value {
    json!({
        "id":                       row["id"],
        "title":                    row["title"],
        "status":                   row["status"],
        "fingerprint":              row["fingerprint"],
        "proposed_solution_summary": row["proposed_solution_summary"],
        "has_proposed_solution":    !row["proposed_files"].is_null(),
        "created_by":               row["created_by"],
        "created_at":               row["created_at"],
        "updated_at":               row["updated_at"],
        "deployment":               json!({ "id": row["deployment_id"], "name": row["deployment_name"] }),
    })
}

pub fn shape_issue(row: &Value) -> Value {
    let proposed_files = row["proposed_files"].as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    json!({
        "id":                        row["id"],
        "title":                     row["title"],
        "description":               row["description"],
        "status":                    row["status"],
        "fingerprint":               row["fingerprint"],
        "proposed_solution_summary": row["proposed_solution_summary"],
        "proposed_files":            proposed_files,
        "comments":                  parse_json_array(row, "comments"),
        "chat_messages":             parse_json_array(row, "chat_messages"),
        "created_by":                row["created_by"],
        "created_at":                row["created_at"],
        "updated_at":                row["updated_at"],
        "deployment": json!({
            "id":          row["deployment_id"],
            "name":        row["deployment_name"],
            "infra_state": row["deployment_infra_state"],
        }),
    })
}

pub async fn group_id_for_project(
    neo4j:      &Neo4jClient,
    project_id: &str,
) -> anyhow::Result<Option<String>> {
    let rows = neo4j.query_read(
        "MATCH (g:Group)-[:HAS_PROJECT]->(:Project {id: $pid}) RETURN g.id AS id",
        json!({ "pid": project_id }),
    ).await?;
    Ok(rows.into_iter().next().and_then(|r| r["id"].as_str().map(str::to_string)))
}

pub async fn append_issue_comment(
    neo4j:       &Neo4jClient,
    project_id:  &str,
    issue_id:    &str,
    author_type: &str,
    author_name: &str,
    body:        &str,
) -> anyhow::Result<()> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         RETURN i.comments AS comments",
        json!({ "pid": project_id, "iid": issue_id }),
    ).await?;
    let Some(row) = rows.into_iter().next() else { return Ok(()) };
    let mut comments: Vec<Value> = row["comments"].as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let now = chrono::Utc::now().to_rfc3339();
    comments.push(json!({
        "id":          uuid::Uuid::new_v4().to_string(),
        "author_type": author_type,
        "author_name": author_name,
        "body":        body,
        "created_at":  now,
    }));
    let comments_json = serde_json::to_string(&comments)?;
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         SET i.comments = $comments, i.updated_at = $now",
        json!({ "pid": project_id, "iid": issue_id, "comments": comments_json, "now": now }),
    ).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn create_issue(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
    run:           &FailedRun,
    title:         &str,
    description:   &str,
    fingerprint:   &str,
    created_by:    &str,
) -> anyhow::Result<String> {
    let id  = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         MATCH (d)-[:HAS_RUN]->(r:DeploymentRun {id: $rid})
         CREATE (i:Issue {
             id: $id, title: $title, description: $description, status: 'untriaged',
             fingerprint: $fingerprint, proposed_solution_summary: null, proposed_files: null,
             chat_messages: '[]', comments: '[]',
             created_by: $created_by, created_at: $now, updated_at: $now
         })
         CREATE (d)-[:HAS_ISSUE]->(i)
         CREATE (i)-[:FROM_RUN]->(r)",
        json!({
            "pid": project_id, "did": deployment_id, "rid": run.id, "id": id,
            "title": title, "description": description, "fingerprint": fingerprint,
            "created_by": created_by, "now": now,
        }),
    ).await?;
    append_issue_comment(neo4j, project_id, &id, "harvest", "Harvest", &issue_created_comment(run)).await?;
    Ok(id)
}

pub async fn link_run_to_issue(
    neo4j:      &Neo4jClient,
    project_id: &str,
    issue_id:   &str,
    run:        &FailedRun,
) -> anyhow::Result<()> {
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         MATCH (d)-[:HAS_RUN]->(r:DeploymentRun {id: $rid})
         MERGE (i)-[:FROM_RUN]->(r)
         SET i.updated_at = $now",
        json!({ "pid": project_id, "iid": issue_id, "rid": run.id, "now": chrono::Utc::now().to_rfc3339() }),
    ).await?;
    append_issue_comment(neo4j, project_id, issue_id, "harvest", "Harvest", &run_linked_comment(run)).await?;
    Ok(())
}

pub async fn reopen_for_regression(
    neo4j:           &Neo4jClient,
    project_id:      &str,
    issue_id:        &str,
    previous_status: IssueStatus,
    run:             &FailedRun,
) -> anyhow::Result<()> {
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         SET i.status = 'untriaged', i.updated_at = $now",
        json!({ "pid": project_id, "iid": issue_id, "now": chrono::Utc::now().to_rfc3339() }),
    ).await?;
    append_issue_comment(
        neo4j, project_id, issue_id, "harvest", "Harvest",
        &regression_reopened_comment(run, previous_status),
    ).await?;
    Ok(())
}

pub async fn issue_status(
    neo4j:      &Neo4jClient,
    project_id: &str,
    issue_id:   &str,
) -> anyhow::Result<Option<IssueStatus>> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         RETURN i.status AS status",
        json!({ "pid": project_id, "iid": issue_id }),
    ).await?;
    Ok(rows.into_iter().next().and_then(|r| IssueStatus::parse(r["status"].as_str().unwrap_or(""))))
}

pub enum UpdateStatusOutcome {
    Applied,
    NotFound,
    Invalid(String),
}

pub async fn update_issue_status(
    neo4j:      &Neo4jClient,
    project_id: &str,
    issue_id:   &str,
    requested:  IssueStatus,
    changed_by: &str,
) -> anyhow::Result<UpdateStatusOutcome> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         RETURN i.status AS status",
        json!({ "pid": project_id, "iid": issue_id }),
    ).await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(UpdateStatusOutcome::NotFound);
    };
    let current = IssueStatus::parse(row["status"].as_str().unwrap_or(""))
        .unwrap_or(IssueStatus::Untriaged);
    if let Err(e) = validate_transition(current, requested) {
        return Ok(UpdateStatusOutcome::Invalid(e));
    }

    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         SET i.status = $status, i.updated_at = $now",
        json!({ "pid": project_id, "iid": issue_id, "status": requested.as_str(), "now": now }),
    ).await?;
    append_issue_comment(
        neo4j, project_id, issue_id, "user", changed_by,
        &status_changed_comment(current, requested, changed_by),
    ).await?;
    Ok(UpdateStatusOutcome::Applied)
}

pub async fn set_proposed_solution(
    neo4j:      &Neo4jClient,
    project_id: &str,
    issue_id:   &str,
    summary:    &str,
    files_json: &str,
    file_count: usize,
) -> anyhow::Result<()> {
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         SET i.proposed_solution_summary = $summary, i.proposed_files = $files, i.updated_at = $now",
        json!({
            "pid": project_id, "iid": issue_id, "summary": summary, "files": files_json,
            "now": chrono::Utc::now().to_rfc3339(),
        }),
    ).await?;
    append_issue_comment(
        neo4j, project_id, issue_id, "harvest", "Harvest",
        &solution_proposed_comment(summary, file_count),
    ).await?;
    Ok(())
}

pub async fn clear_proposed_solution_and_record_apply(
    neo4j:      &Neo4jClient,
    project_id: &str,
    issue_id:   &str,
    summary:    &str,
) -> anyhow::Result<()> {
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         SET i.proposed_solution_summary = null, i.proposed_files = null, i.updated_at = $now",
        json!({ "pid": project_id, "iid": issue_id, "now": chrono::Utc::now().to_rfc3339() }),
    ).await?;
    append_issue_comment(
        neo4j, project_id, issue_id, "harvest", "Harvest",
        &solution_applied_comment(summary),
    ).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn append_issue_chat_turn(
    neo4j:           &Neo4jClient,
    project_id:      &str,
    issue_id:        &str,
    user_text:       &str,
    username:        &str,
    assistant_text:  &str,
    chain:           Vec<Value>,
    tool_calls_made: usize,
) -> anyhow::Result<()> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         RETURN i.chat_messages AS chat_messages",
        json!({ "pid": project_id, "iid": issue_id }),
    ).await?;
    let Some(row) = rows.into_iter().next() else { return Ok(()) };
    let mut messages: Vec<Value> = row["chat_messages"].as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    messages.push(json!({ "role": "user", "text": user_text, "username": username }));
    messages.push(json!({
        "role": "assistant", "text": assistant_text, "chain": chain,
        "tool_calls_made": tool_calls_made,
    }));
    let messages_json = serde_json::to_string(&messages)?;
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         SET i.chat_messages = $messages, i.updated_at = $now",
        json!({
            "pid": project_id, "iid": issue_id, "messages": messages_json,
            "now": chrono::Utc::now().to_rfc3339(),
        }),
    ).await?;
    Ok(())
}

pub async fn list_issues_for_project(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    status:        Option<&str>,
    deployment_id: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)-[:HAS_ISSUE]->(i:Issue)
         WHERE ($status IS NULL OR i.status = $status) AND ($did IS NULL OR d.id = $did)
         RETURN i.id AS id, i.title AS title, i.status AS status, i.fingerprint AS fingerprint,
                i.proposed_solution_summary AS proposed_solution_summary, i.proposed_files AS proposed_files,
                i.created_by AS created_by, i.created_at AS created_at, i.updated_at AS updated_at,
                d.id AS deployment_id, d.name AS deployment_name
         ORDER BY i.updated_at DESC",
        json!({ "pid": project_id, "status": status, "did": deployment_id }),
    ).await?;
    Ok(rows.iter().map(shape_issue_summary).collect())
}

pub async fn get_issue_detail(
    neo4j:      &Neo4jClient,
    project_id: &str,
    issue_id:   &str,
) -> anyhow::Result<Option<Value>> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)-[:HAS_ISSUE]->(i:Issue {id: $iid})
         RETURN i.id AS id, i.title AS title, i.description AS description, i.status AS status,
                i.fingerprint AS fingerprint, i.proposed_solution_summary AS proposed_solution_summary,
                i.proposed_files AS proposed_files, i.chat_messages AS chat_messages, i.comments AS comments,
                i.created_by AS created_by, i.created_at AS created_at, i.updated_at AS updated_at,
                d.id AS deployment_id, d.name AS deployment_name, d.infra_state AS deployment_infra_state",
        json!({ "pid": project_id, "iid": issue_id }),
    ).await?;
    let Some(row) = rows.into_iter().next() else { return Ok(None) };
    let mut shaped = shape_issue(&row);

    let run_rows = neo4j.query_read(
        "MATCH (:Issue {id: $iid})-[:FROM_RUN]->(r:DeploymentRun)
         RETURN r.id AS id, r.action AS action, r.status AS status, r.exit_code AS exit_code,
                r.stdout_preview AS stdout_preview, r.stderr_preview AS stderr_preview,
                r.initiated_by AS initiated_by, r.reasoning AS reasoning, r.created_at AS created_at
         ORDER BY r.created_at DESC",
        json!({ "iid": issue_id }),
    ).await?;
    shaped["runs"] = Value::Array(run_rows);
    Ok(Some(shaped))
}

pub async fn list_open_issues_for_deployment(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
) -> anyhow::Result<Vec<Value>> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment {id: $did})-[:HAS_ISSUE]->(i:Issue)
         RETURN i.id AS id, i.title AS title, i.description AS description, i.status AS status,
                i.fingerprint AS fingerprint
         ORDER BY i.updated_at DESC",
        json!({ "pid": project_id, "did": deployment_id }),
    ).await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_status_parse_and_as_str_round_trip() {
        for status in [
            IssueStatus::Untriaged, IssueStatus::InProgress, IssueStatus::Fixed, IssueStatus::Rejected,
        ] {
            assert_eq!(IssueStatus::parse(status.as_str()), Some(status));
        }
    }

    #[test]
    fn issue_status_parse_rejects_unknown_string() {
        assert_eq!(IssueStatus::parse("nonsense"), None);
    }

    #[test]
    fn validate_transition_allows_untriaged_to_in_progress() {
        assert!(validate_transition(IssueStatus::Untriaged, IssueStatus::InProgress).is_ok());
    }

    #[test]
    fn validate_transition_allows_in_progress_to_fixed() {
        assert!(validate_transition(IssueStatus::InProgress, IssueStatus::Fixed).is_ok());
    }

    #[test]
    fn validate_transition_allows_in_progress_to_rejected() {
        assert!(validate_transition(IssueStatus::InProgress, IssueStatus::Rejected).is_ok());
    }

    #[test]
    fn validate_transition_rejects_no_ops() {
        assert!(validate_transition(IssueStatus::Untriaged, IssueStatus::Untriaged).is_err());
        assert!(validate_transition(IssueStatus::InProgress, IssueStatus::InProgress).is_err());
        assert!(validate_transition(IssueStatus::Fixed, IssueStatus::Fixed).is_err());
    }

    #[test]
    fn validate_transition_rejects_backward_moves() {
        assert!(validate_transition(IssueStatus::Fixed, IssueStatus::InProgress).is_err());
        assert!(validate_transition(IssueStatus::Rejected, IssueStatus::InProgress).is_err());
        assert!(validate_transition(IssueStatus::InProgress, IssueStatus::Untriaged).is_err());
    }

    #[test]
    fn validate_transition_rejects_skips() {
        assert!(validate_transition(IssueStatus::Untriaged, IssueStatus::Fixed).is_err());
        assert!(validate_transition(IssueStatus::Untriaged, IssueStatus::Rejected).is_err());
    }

    #[test]
    fn validate_transition_rejects_moves_out_of_terminal_states() {
        assert!(validate_transition(IssueStatus::Fixed, IssueStatus::Rejected).is_err());
        assert!(validate_transition(IssueStatus::Rejected, IssueStatus::Fixed).is_err());
    }

    #[test]
    fn is_regression_reopen_true_only_for_fixed_or_rejected() {
        assert!(!is_regression_reopen(IssueStatus::Untriaged));
        assert!(!is_regression_reopen(IssueStatus::InProgress));
        assert!(is_regression_reopen(IssueStatus::Fixed));
        assert!(is_regression_reopen(IssueStatus::Rejected));
    }

    fn sample_run() -> FailedRun {
        FailedRun {
            id:             "run-42".into(),
            action:         "apply".into(),
            exit_code:      Some(1),
            stdout_preview: "".into(),
            stderr_preview: "Error: connection refused on port 8443".into(),
        }
    }

    #[test]
    fn issue_created_comment_includes_run_id_action_and_snippet() {
        let comment = issue_created_comment(&sample_run());
        assert!(comment.contains("run-42"));
        assert!(comment.contains("apply"));
        assert!(comment.contains("connection refused on port 8443"));
    }

    #[test]
    fn run_linked_comment_includes_run_id_and_snippet() {
        let comment = run_linked_comment(&sample_run());
        assert!(comment.contains("run-42"));
        assert!(comment.contains("connection refused on port 8443"));
    }

    #[test]
    fn regression_reopened_comment_includes_previous_status_run_id_and_snippet() {
        let comment = regression_reopened_comment(&sample_run(), IssueStatus::Fixed);
        assert!(comment.contains("fixed"));
        assert!(comment.contains("run-42"));
        assert!(comment.contains("connection refused on port 8443"));
    }

    #[test]
    fn status_changed_comment_includes_both_statuses_and_actor() {
        let comment = status_changed_comment(IssueStatus::Untriaged, IssueStatus::InProgress, "Alice");
        assert!(comment.contains("Alice"));
        assert!(comment.contains("untriaged"));
        assert!(comment.contains("in_progress"));
    }

    #[test]
    fn solution_proposed_comment_includes_file_count_and_summary() {
        let comment = solution_proposed_comment("fixed the security group", 3);
        assert!(comment.contains('3'));
        assert!(comment.contains("fixed the security group"));
    }

    #[test]
    fn solution_applied_comment_includes_summary() {
        let comment = solution_applied_comment("widened the security group rule");
        assert!(comment.contains("widened the security group rule"));
    }

    fn base_issue_row() -> Value {
        json!({
            "id": "i1", "title": "Apply fails on security group", "description": "details",
            "status": "untriaged", "fingerprint": "sg-conflict",
            "proposed_solution_summary": Value::Null, "proposed_files": Value::Null,
            "comments": "[]", "chat_messages": "[]",
            "created_by": "harvest", "created_at": "t1", "updated_at": "t1",
            "deployment_id": "d1", "deployment_name": "Acme rollout", "deployment_infra_state": "broken",
        })
    }

    #[test]
    fn shape_issue_passes_through_scalar_fields() {
        let shaped = shape_issue(&base_issue_row());
        assert_eq!(shaped["id"], "i1");
        assert_eq!(shaped["title"], "Apply fails on security group");
        assert_eq!(shaped["status"], "untriaged");
    }

    #[test]
    fn shape_issue_parses_proposed_files_json_blob() {
        let mut row = base_issue_row();
        row["proposed_files"] = json!("{\"main.tf\": \"resource...\"}");
        let shaped = shape_issue(&row);
        assert_eq!(shaped["proposed_files"]["main.tf"], "resource...");
    }

    #[test]
    fn shape_issue_proposed_files_null_when_absent() {
        let shaped = shape_issue(&base_issue_row());
        assert_eq!(shaped["proposed_files"], Value::Null);
    }

    #[test]
    fn shape_issue_parses_comments_and_chat_messages_json_blobs() {
        let mut row = base_issue_row();
        row["comments"] = json!("[{\"author_type\": \"harvest\", \"body\": \"hi\"}]");
        row["chat_messages"] = json!("[{\"role\": \"user\", \"text\": \"hello\"}]");
        let shaped = shape_issue(&row);
        assert_eq!(shaped["comments"][0]["body"], "hi");
        assert_eq!(shaped["chat_messages"][0]["text"], "hello");
    }

    #[test]
    fn shape_issue_defaults_comments_and_chat_messages_to_empty_arrays() {
        let mut row = base_issue_row();
        row["comments"] = Value::Null;
        row["chat_messages"] = Value::Null;
        let shaped = shape_issue(&row);
        assert_eq!(shaped["comments"], json!([]));
        assert_eq!(shaped["chat_messages"], json!([]));
    }

    #[test]
    fn shape_issue_nests_deployment_fields() {
        let shaped = shape_issue(&base_issue_row());
        assert_eq!(shaped["deployment"]["id"], "d1");
        assert_eq!(shaped["deployment"]["name"], "Acme rollout");
        assert_eq!(shaped["deployment"]["infra_state"], "broken");
    }

    #[test]
    fn shape_issue_summary_has_proposed_solution_false_when_absent() {
        let shaped = shape_issue_summary(&base_issue_row());
        assert_eq!(shaped["has_proposed_solution"], false);
    }

    #[test]
    fn shape_issue_summary_has_proposed_solution_true_when_present() {
        let mut row = base_issue_row();
        row["proposed_files"] = json!("{\"main.tf\": \"x\"}");
        let shaped = shape_issue_summary(&row);
        assert_eq!(shaped["has_proposed_solution"], true);
    }

    #[test]
    fn shape_issue_summary_omits_description_and_comments() {
        let shaped = shape_issue_summary(&base_issue_row());
        assert!(shaped.get("description").is_none());
        assert!(shaped.get("comments").is_none());
    }
}

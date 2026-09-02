use base64::{engine::general_purpose::STANDARD, Engine as _};
use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use crate::neo4j::Neo4jClient;

static RUNNING: LazyLock<DashMap<String, ()>> = LazyLock::new(DashMap::new);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignVersion {
    pub artifact_id: String,
    pub updated_at:  String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyCache {
    pub version: DesignVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedAttempt {
    pub version: DesignVersion,
    pub error:   String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PdfCacheState {
    pub ready:  Option<ReadyCache>,
    pub failed: Option<FailedAttempt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeDecision {
    ServeReady,
    ServeStale { trigger: bool },
    WaitForGeneration { trigger: bool },
    ServeError,
}

pub fn decide_serve(cache: &PdfCacheState, current: &DesignVersion, is_regenerating: bool) -> ServeDecision {
    if let Some(ready) = &cache.ready {
        if ready.version == *current {
            return ServeDecision::ServeReady;
        }
        return ServeDecision::ServeStale { trigger: !is_regenerating };
    }
    if let Some(failed) = &cache.failed {
        if failed.version == *current && !is_regenerating {
            return ServeDecision::ServeError;
        }
    }
    ServeDecision::WaitForGeneration { trigger: !is_regenerating }
}

pub struct RenderInput {
    pub version:         DesignVersion,
    pub title:           String,
    pub content:         String,
    pub company:         String,
    pub product:         String,
    pub deployment_name: String,
}

fn opt_str(row: &Value, key: &str) -> Option<String> {
    row[key].as_str().map(str::to_string)
}

pub async fn load_render_input(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
) -> anyhow::Result<Option<RenderInput>> {
    let rows = neo4j.query_read(
        "MATCH (p:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         OPTIONAL MATCH (d)-[:USES_TEMPLATE]->(t:ProductTemplate)
         OPTIONAL MATCH (d)-[:HAS_DESIGN_DOC]->(a:Artifact)
         RETURN p.name AS project_name, d.name AS deployment_name, t.name AS template_name,
                a.id AS artifact_id, a.title AS artifact_title, a.content AS artifact_content,
                a.updated_at AS artifact_updated_at",
        json!({ "pid": project_id, "did": deployment_id }),
    ).await?;
    let Some(row) = rows.into_iter().next() else { return Ok(None) };
    let Some(artifact_id) = opt_str(&row, "artifact_id") else { return Ok(None) };
    let Some(updated_at) = opt_str(&row, "artifact_updated_at") else { return Ok(None) };
    Ok(Some(RenderInput {
        version:         DesignVersion { artifact_id, updated_at },
        title:           opt_str(&row, "artifact_title").unwrap_or_else(|| "Design".to_string()),
        content:         opt_str(&row, "artifact_content").unwrap_or_default(),
        company:         opt_str(&row, "project_name").unwrap_or_default(),
        product:         opt_str(&row, "template_name").unwrap_or_default(),
        deployment_name: opt_str(&row, "deployment_name").unwrap_or_default(),
    }))
}

pub async fn load_cache_state(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
) -> anyhow::Result<PdfCacheState> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         RETURN d.design_pdf_ready_artifact_id AS ready_artifact_id,
                d.design_pdf_ready_artifact_updated_at AS ready_artifact_updated_at,
                d.design_pdf_failed_artifact_id AS failed_artifact_id,
                d.design_pdf_failed_artifact_updated_at AS failed_artifact_updated_at,
                d.design_pdf_failed_error AS failed_error",
        json!({ "pid": project_id, "did": deployment_id }),
    ).await?;
    let Some(row) = rows.into_iter().next() else { return Ok(PdfCacheState::default()) };

    let ready = match (opt_str(&row, "ready_artifact_id"), opt_str(&row, "ready_artifact_updated_at")) {
        (Some(artifact_id), Some(updated_at)) => Some(ReadyCache { version: DesignVersion { artifact_id, updated_at } }),
        _ => None,
    };
    let failed = match (
        opt_str(&row, "failed_artifact_id"),
        opt_str(&row, "failed_artifact_updated_at"),
        opt_str(&row, "failed_error"),
    ) {
        (Some(artifact_id), Some(updated_at), Some(error)) =>
            Some(FailedAttempt { version: DesignVersion { artifact_id, updated_at }, error }),
        _ => None,
    };
    Ok(PdfCacheState { ready, failed })
}

pub async fn load_ready_bytes(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         RETURN d.design_pdf_ready_bytes AS bytes",
        json!({ "pid": project_id, "did": deployment_id }),
    ).await?;
    let Some(b64) = rows.into_iter().next().and_then(|row| opt_str(&row, "bytes")) else { return Ok(None) };
    Ok(Some(STANDARD.decode(b64)?))
}

pub async fn store_ready(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
    version:       &DesignVersion,
    bytes:         &[u8],
) -> anyhow::Result<()> {
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         SET d.design_pdf_ready_artifact_id = $aid,
             d.design_pdf_ready_artifact_updated_at = $updated_at,
             d.design_pdf_ready_bytes = $bytes,
             d.design_pdf_ready_generated_at = $now",
        json!({
            "pid": project_id, "did": deployment_id,
            "aid": version.artifact_id, "updated_at": version.updated_at,
            "bytes": STANDARD.encode(bytes), "now": chrono::Utc::now().to_rfc3339(),
        }),
    ).await?;
    Ok(())
}

pub async fn store_failed(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
    version:       &DesignVersion,
    error:         &str,
) -> anyhow::Result<()> {
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         SET d.design_pdf_failed_artifact_id = $aid,
             d.design_pdf_failed_artifact_updated_at = $updated_at,
             d.design_pdf_failed_error = $error",
        json!({
            "pid": project_id, "did": deployment_id,
            "aid": version.artifact_id, "updated_at": version.updated_at, "error": error,
        }),
    ).await?;
    Ok(())
}

fn is_running(deployment_id: &str) -> bool {
    RUNNING.contains_key(deployment_id)
}

async fn wait_until_idle(deployment_id: &str, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    while is_running(deployment_id) {
        if tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

async fn run_regeneration(neo4j: &Neo4jClient, project_id: &str, deployment_id: &str) {
    let input = match load_render_input(neo4j, project_id, deployment_id).await {
        Ok(Some(input)) => input,
        _ => return,
    };
    let info = super::design_pdf::TitlePageInfo {
        company:         input.company,
        product:         input.product,
        deployment_name: input.deployment_name,
        generated_date:  chrono::Utc::now().format("%Y-%m-%d").to_string(),
    };
    match super::design_pdf::build_design_pdf(&input.content, &info) {
        Ok(bytes) => { let _ = store_ready(neo4j, project_id, deployment_id, &input.version, &bytes).await; }
        Err(e)    => { let _ = store_failed(neo4j, project_id, deployment_id, &input.version, &e).await; }
    }
}

pub fn schedule_regeneration(neo4j: Arc<Neo4jClient>, project_id: String, deployment_id: String) {
    if RUNNING.insert(deployment_id.clone(), ()).is_some() {
        return;
    }
    tokio::spawn(async move {
        run_regeneration(&neo4j, &project_id, &deployment_id).await;
        RUNNING.remove(&deployment_id);
    });
}

pub async fn on_artifact_changed(
    neo4j:      Arc<Neo4jClient>,
    project_id: String,
    artifact_id: String,
) -> anyhow::Result<()> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)-[:HAS_DESIGN_DOC]->(:Artifact {id: $aid})
         RETURN d.id AS id",
        json!({ "pid": project_id, "aid": artifact_id }),
    ).await?;
    for row in rows {
        if let Some(deployment_id) = opt_str(&row, "id") {
            schedule_regeneration(neo4j.clone(), project_id.clone(), deployment_id);
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum ResolvedPdf {
    Bytes { data: Vec<u8>, stale: bool },
    Pending,
    Failed(String),
    NoDesignDoc,
}

pub async fn resolve_for_serving(
    neo4j:         Arc<Neo4jClient>,
    project_id:    String,
    deployment_id: String,
) -> anyhow::Result<ResolvedPdf> {
    let Some(input) = load_render_input(&neo4j, &project_id, &deployment_id).await? else {
        return Ok(ResolvedPdf::NoDesignDoc);
    };
    let cache = load_cache_state(&neo4j, &project_id, &deployment_id).await?;

    match decide_serve(&cache, &input.version, is_running(&deployment_id)) {
        ServeDecision::ServeReady => {
            let bytes = load_ready_bytes(&neo4j, &project_id, &deployment_id).await?.unwrap_or_default();
            Ok(ResolvedPdf::Bytes { data: bytes, stale: false })
        }
        ServeDecision::ServeStale { trigger } => {
            if trigger {
                schedule_regeneration(neo4j.clone(), project_id.clone(), deployment_id.clone());
            }
            let bytes = load_ready_bytes(&neo4j, &project_id, &deployment_id).await?.unwrap_or_default();
            Ok(ResolvedPdf::Bytes { data: bytes, stale: true })
        }
        ServeDecision::WaitForGeneration { trigger } => {
            if trigger {
                schedule_regeneration(neo4j.clone(), project_id.clone(), deployment_id.clone());
            }
            wait_until_idle(&deployment_id, Duration::from_secs(30)).await;
            let cache = load_cache_state(&neo4j, &project_id, &deployment_id).await?;
            match decide_serve(&cache, &input.version, is_running(&deployment_id)) {
                ServeDecision::ServeReady => {
                    let bytes = load_ready_bytes(&neo4j, &project_id, &deployment_id).await?.unwrap_or_default();
                    Ok(ResolvedPdf::Bytes { data: bytes, stale: false })
                }
                ServeDecision::ServeError => Ok(ResolvedPdf::Failed(cache.failed.map(|f| f.error).unwrap_or_default())),
                _ => Ok(ResolvedPdf::Pending),
            }
        }
        ServeDecision::ServeError => Ok(ResolvedPdf::Failed(cache.failed.map(|f| f.error).unwrap_or_default())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(id: &str, ts: &str) -> DesignVersion {
        DesignVersion { artifact_id: id.to_string(), updated_at: ts.to_string() }
    }

    #[test]
    fn no_prior_cache_waits_and_triggers() {
        let cache = PdfCacheState::default();
        let current = version("a1", "t1");
        assert_eq!(decide_serve(&cache, &current, false), ServeDecision::WaitForGeneration { trigger: true });
    }

    #[test]
    fn no_prior_cache_but_already_regenerating_waits_without_retriggering() {
        let cache = PdfCacheState::default();
        let current = version("a1", "t1");
        assert_eq!(decide_serve(&cache, &current, true), ServeDecision::WaitForGeneration { trigger: false });
    }

    #[test]
    fn ready_cache_matching_current_version_serves_without_triggering() {
        let cache = PdfCacheState { ready: Some(ReadyCache { version: version("a1", "t1") }), failed: None };
        let current = version("a1", "t1");
        assert_eq!(decide_serve(&cache, &current, false), ServeDecision::ServeReady);
    }

    #[test]
    fn ready_cache_stale_content_serves_stale_and_triggers_regeneration() {
        let cache = PdfCacheState { ready: Some(ReadyCache { version: version("a1", "t1") }), failed: None };
        let current = version("a1", "t2");
        assert_eq!(decide_serve(&cache, &current, false), ServeDecision::ServeStale { trigger: true });
    }

    #[test]
    fn ready_cache_stale_but_regeneration_already_in_flight_serves_stale_without_retriggering() {
        let cache = PdfCacheState { ready: Some(ReadyCache { version: version("a1", "t1") }), failed: None };
        let current = version("a1", "t2");
        assert_eq!(decide_serve(&cache, &current, true), ServeDecision::ServeStale { trigger: false });
    }

    #[test]
    fn ready_cache_stale_due_to_different_artifact_id_still_serves_stale() {
        let cache = PdfCacheState { ready: Some(ReadyCache { version: version("a1", "t1") }), failed: None };
        let current = version("a2", "t1");
        assert_eq!(decide_serve(&cache, &current, false), ServeDecision::ServeStale { trigger: true });
    }

    #[test]
    fn failed_attempt_matching_current_version_serves_error_without_retriggering() {
        let cache = PdfCacheState { ready: None, failed: Some(FailedAttempt { version: version("a1", "t1"), error: "boom".into() }) };
        let current = version("a1", "t1");
        assert_eq!(decide_serve(&cache, &current, false), ServeDecision::ServeError);
    }

    #[test]
    fn failed_attempt_for_older_version_is_retried_for_new_content() {
        let cache = PdfCacheState { ready: None, failed: Some(FailedAttempt { version: version("a1", "t1"), error: "boom".into() }) };
        let current = version("a1", "t2");
        assert_eq!(decide_serve(&cache, &current, false), ServeDecision::WaitForGeneration { trigger: true });
    }

    #[test]
    fn failed_attempt_matching_current_version_but_already_regenerating_waits() {
        let cache = PdfCacheState { ready: None, failed: Some(FailedAttempt { version: version("a1", "t1"), error: "boom".into() }) };
        let current = version("a1", "t1");
        assert_eq!(decide_serve(&cache, &current, true), ServeDecision::WaitForGeneration { trigger: false });
    }

    #[test]
    fn ready_cache_takes_precedence_over_stale_failed_record() {
        let cache = PdfCacheState {
            ready:  Some(ReadyCache { version: version("a1", "t2") }),
            failed: Some(FailedAttempt { version: version("a1", "t1"), error: "boom".into() }),
        };
        let current = version("a1", "t2");
        assert_eq!(decide_serve(&cache, &current, false), ServeDecision::ServeReady);
    }
}

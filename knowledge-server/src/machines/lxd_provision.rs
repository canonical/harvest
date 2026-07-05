use anyhow::{bail, Result};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::lxd::{CreateInstance, Flavor, LxdClient};
use crate::neo4j::Neo4jClient;

use super::handlers::{generate_install_script, get_or_create_install_token};
use super::hash_token;

const MAX_INSTANCE_NAME_LEN: usize = 63;
const INSTANCE_SUFFIX_LEN: usize = 5;
const NETWORK_HASH_LEN: usize = 10;
const WAIT_RUNNING_TIMEOUT_SECS: u64 = 60;
const EXEC_INSTALL_ATTEMPTS: u32 = 4;
const EXEC_INSTALL_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionPhase {
    EnsureNetwork,
    InstallToken,
    CreateContainer,
    StartContainer,
    WaitRunning,
    InstallAgent,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProvisionEvent {
    PhaseStart { phase: ProvisionPhase },
    InstallRetry { attempt: u32, attempts: u32 },
    Done { hostname: String },
    Error { phase: ProvisionPhase, message: String },
}

async fn emit(progress: &mpsc::Sender<String>, event: ProvisionEvent) {
    if let Ok(data) = serde_json::to_string(&event) {
        let _ = progress.send(data).await;
    }
}

pub fn network_name_for_project(project_id: &str) -> String {
    let hash = hash_token(project_id);
    format!("hv-{}", &hash[..NETWORK_HASH_LEN])
}

pub fn sanitize_name_base(input: &str) -> String {
    let lowered: String = input
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    let mut collapsed = lowered;
    while collapsed.contains("--") {
        collapsed = collapsed.replace("--", "-");
    }
    let trimmed = collapsed.trim_matches('-');

    let based = match trimmed.chars().next() {
        Some(c) if c.is_ascii_alphabetic() => trimmed.to_string(),
        _ => format!("agent-{trimmed}"),
    };

    let max_base_len = MAX_INSTANCE_NAME_LEN - INSTANCE_SUFFIX_LEN;
    if based.len() > max_base_len {
        based[..max_base_len].trim_end_matches('-').to_string()
    } else {
        based
    }
}

pub fn unique_instance_name(user_name: &str) -> String {
    let base = sanitize_name_base(user_name);
    let suffix: String = Uuid::new_v4().simple().to_string().chars().take(4).collect();
    format!("{base}-{suffix}")
}

async fn cleanup_failed_provision(
    lxd: &LxdClient,
    neo4j: &Neo4jClient,
    project_id: &str,
    instance_name: &str,
) {
    tracing::warn!(instance_name, "cleaning up failed LXD agent provisioning attempt");
    if let Err(e) = lxd.delete_instance(instance_name).await {
        tracing::warn!(instance_name, error = ?e, "cleanup: failed to delete LXD instance (it may never have been created)");
    }
    if let Err(e) = neo4j.query_read(
        "MATCH (li:LxdInstance {project_id: $pid, hostname: $h}) DELETE li",
        json!({ "pid": project_id, "h": instance_name }),
    ).await {
        tracing::warn!(instance_name, error = ?e, "cleanup: failed to delete LxdInstance marker node");
    }
}

pub async fn create_lxd_agent(
    neo4j:      &Neo4jClient,
    lxd:        &LxdClient,
    server_url: &str,
    project_id: &str,
    name:       &str,
    description: &str,
    flavor:     Flavor,
    progress:   mpsc::Sender<String>,
) -> Result<Value> {
    let network = network_name_for_project(project_id);
    tracing::info!(project_id, name, flavor = flavor.id(), network, "provisioning LXD-managed agent: step 1/6 ensure network");
    emit(&progress, ProvisionEvent::PhaseStart { phase: ProvisionPhase::EnsureNetwork }).await;

    if let Err(e) = lxd.ensure_network(&network).await {
        tracing::error!(project_id, network, error = ?e, "step 1/6 FAILED: ensure_network");
        emit(&progress, ProvisionEvent::Error { phase: ProvisionPhase::EnsureNetwork, message: format!("{e:#}") }).await;
        return Err(e);
    }

    tracing::info!(project_id, "step 2/6 get or create install token");
    emit(&progress, ProvisionEvent::PhaseStart { phase: ProvisionPhase::InstallToken }).await;
    let install_token = match get_or_create_install_token(neo4j, project_id).await {
        Ok(Some(tok)) => tok,
        Ok(None) => {
            tracing::error!(project_id, "step 2/6 FAILED: project not found");
            emit(&progress, ProvisionEvent::Error { phase: ProvisionPhase::InstallToken, message: "project not found".into() }).await;
            bail!("project not found");
        }
        Err(e) => {
            tracing::error!(project_id, error = ?e, "step 2/6 FAILED: get_or_create_install_token");
            emit(&progress, ProvisionEvent::Error { phase: ProvisionPhase::InstallToken, message: format!("{e:#}") }).await;
            return Err(e);
        }
    };
    let script = generate_install_script(server_url, &install_token);

    let instance_name = unique_instance_name(name);
    let now = chrono::Utc::now().to_rfc3339();

    tracing::info!(instance_name, "step 3/6 write LxdInstance marker");
    if let Err(e) = neo4j.query_read(
        "CREATE (:LxdInstance {
             project_id: $pid, hostname: $h, lxd_project: $lp,
             description: $desc, created_at: $now
         })",
        json!({
            "pid": project_id, "h": instance_name, "lp": lxd.project(),
            "desc": description, "now": now,
        }),
    ).await {
        tracing::error!(project_id, instance_name, error = ?e, "step 3/6 FAILED: write LxdInstance marker");
        emit(&progress, ProvisionEvent::Error { phase: ProvisionPhase::InstallToken, message: format!("{e:#}") }).await;
        return Err(e.into());
    }

    let create_req = CreateInstance {
        name:        instance_name.clone(),
        network:     network.clone(),
        description: description.to_string(),
        flavor,
    };

    tracing::info!(instance_name, network, flavor = flavor.id(), "step 4/6 create LXD container");
    emit(&progress, ProvisionEvent::PhaseStart { phase: ProvisionPhase::CreateContainer }).await;
    if let Err(e) = lxd.create_instance(&create_req).await {
        tracing::error!(instance_name, error = ?e, "step 4/6 FAILED: create_instance");
        emit(&progress, ProvisionEvent::Error { phase: ProvisionPhase::CreateContainer, message: format!("{e:#}") }).await;
        cleanup_failed_provision(lxd, neo4j, project_id, &instance_name).await;
        return Err(e);
    }

    emit(&progress, ProvisionEvent::PhaseStart { phase: ProvisionPhase::StartContainer }).await;
    if let Err(e) = lxd.start_instance(&instance_name).await {
        tracing::error!(instance_name, error = ?e, "step 4/6 FAILED: start_instance");
        emit(&progress, ProvisionEvent::Error { phase: ProvisionPhase::StartContainer, message: format!("{e:#}") }).await;
        cleanup_failed_provision(lxd, neo4j, project_id, &instance_name).await;
        return Err(e);
    }

    emit(&progress, ProvisionEvent::PhaseStart { phase: ProvisionPhase::WaitRunning }).await;
    if let Err(e) = lxd.wait_running(&instance_name, WAIT_RUNNING_TIMEOUT_SECS).await {
        tracing::error!(instance_name, error = ?e, "step 4/6 FAILED: wait_running");
        emit(&progress, ProvisionEvent::Error { phase: ProvisionPhase::WaitRunning, message: format!("{e:#}") }).await;
        cleanup_failed_provision(lxd, neo4j, project_id, &instance_name).await;
        return Err(e);
    }

    tracing::info!(instance_name, "step 5/6 exec install script inside container");
    emit(&progress, ProvisionEvent::PhaseStart { phase: ProvisionPhase::InstallAgent }).await;
    let retry_progress = progress.clone();
    let exit_code = match lxd.exec_with_retry(
        &instance_name, vec!["bash".into(), "-c".into(), script],
        EXEC_INSTALL_ATTEMPTS, EXEC_INSTALL_RETRY_DELAY,
        move |attempt, attempts| {
            let tx = retry_progress.clone();
            let data = serde_json::to_string(&ProvisionEvent::InstallRetry { attempt, attempts }).unwrap_or_default();
            tokio::spawn(async move { let _ = tx.send(data).await; });
        },
    ).await {
        Ok(code) => code,
        Err(e) => {
            tracing::error!(instance_name, error = ?e, "step 5/6 FAILED: exec");
            emit(&progress, ProvisionEvent::Error { phase: ProvisionPhase::InstallAgent, message: format!("{e:#}") }).await;
            cleanup_failed_provision(lxd, neo4j, project_id, &instance_name).await;
            return Err(e);
        }
    };
    if exit_code != 0 {
        tracing::error!(instance_name, exit_code, "step 5/6 FAILED: install script exited non-zero");
        emit(&progress, ProvisionEvent::Error { phase: ProvisionPhase::InstallAgent, message: format!("install script exited with status {exit_code}") }).await;
        cleanup_failed_provision(lxd, neo4j, project_id, &instance_name).await;
        bail!("agent install script exited with status {exit_code}");
    }

    tracing::info!(instance_name, "step 6/6 done — waiting for agent to self-register over SSE");
    emit(&progress, ProvisionEvent::Done { hostname: instance_name.clone() }).await;

    Ok(json!({ "hostname": instance_name, "status": "provisioning" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_lowercases_and_replaces_invalid_chars() {
        assert_eq!(sanitize_name_base("My Agent!"), "my-agent");
    }

    #[test]
    fn sanitize_collapses_repeated_hyphens() {
        assert_eq!(sanitize_name_base("foo   bar"), "foo-bar");
    }

    #[test]
    fn sanitize_trims_leading_and_trailing_hyphens() {
        assert_eq!(sanitize_name_base("--foo--"), "foo");
    }

    #[test]
    fn sanitize_prefixes_when_starting_with_digit() {
        assert_eq!(sanitize_name_base("123-runner"), "agent-123-runner");
    }

    #[test]
    fn sanitize_prefixes_when_empty() {
        assert_eq!(sanitize_name_base("!!!"), "agent-");
    }

    #[test]
    fn sanitize_truncates_long_names() {
        let long = "a".repeat(100);
        let sanitized = sanitize_name_base(&long);
        assert!(sanitized.len() <= MAX_INSTANCE_NAME_LEN - INSTANCE_SUFFIX_LEN);
    }

    #[test]
    fn unique_instance_name_has_suffix_and_fits_lxd_limit() {
        let name = unique_instance_name("My Agent");
        assert!(name.starts_with("my-agent-"));
        assert!(name.len() <= MAX_INSTANCE_NAME_LEN);
    }

    #[test]
    fn unique_instance_name_differs_across_calls() {
        let a = unique_instance_name("same-name");
        let b = unique_instance_name("same-name");
        assert_ne!(a, b);
    }

    #[test]
    fn network_name_is_deterministic() {
        assert_eq!(network_name_for_project("proj-1"), network_name_for_project("proj-1"));
    }

    #[test]
    fn network_name_differs_across_projects() {
        assert_ne!(network_name_for_project("proj-1"), network_name_for_project("proj-2"));
    }

    #[test]
    fn network_name_fits_ifnamsiz_limit() {
        let name = network_name_for_project("a-very-long-project-id-1234567890");
        assert!(name.len() <= 15, "network name too long for IFNAMSIZ: {name} ({} chars)", name.len());
    }

    #[test]
    fn phase_start_event_serializes_with_snake_case_phase() {
        let event = ProvisionEvent::PhaseStart { phase: ProvisionPhase::EnsureNetwork };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "phase_start");
        assert_eq!(json["phase"], "ensure_network");
    }

    #[test]
    fn all_phases_serialize_to_snake_case() {
        let expected = [
            (ProvisionPhase::EnsureNetwork, "ensure_network"),
            (ProvisionPhase::InstallToken, "install_token"),
            (ProvisionPhase::CreateContainer, "create_container"),
            (ProvisionPhase::StartContainer, "start_container"),
            (ProvisionPhase::WaitRunning, "wait_running"),
            (ProvisionPhase::InstallAgent, "install_agent"),
        ];
        for (phase, name) in expected {
            assert_eq!(serde_json::to_value(phase).unwrap(), json!(name));
        }
    }

    #[test]
    fn install_retry_event_serializes_attempt_fields() {
        let event = ProvisionEvent::InstallRetry { attempt: 2, attempts: 4 };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "install_retry");
        assert_eq!(json["attempt"], 2);
        assert_eq!(json["attempts"], 4);
    }

    #[test]
    fn done_event_serializes_hostname() {
        let event = ProvisionEvent::Done { hostname: "agent-abcd".into() };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "done");
        assert_eq!(json["hostname"], "agent-abcd");
    }

    #[test]
    fn error_event_serializes_phase_and_message() {
        let event = ProvisionEvent::Error { phase: ProvisionPhase::InstallAgent, message: "boom".into() };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["phase"], "install_agent");
        assert_eq!(json["message"], "boom");
    }
}

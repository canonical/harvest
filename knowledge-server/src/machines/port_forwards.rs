use chrono::Utc;
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::OnceLock;
use uuid::Uuid;

use crate::neo4j::Neo4jClient;

const RESERVED_ROUTE_NAMES: &[&str] = &["install.sh"];
const MAX_ROUTE_NAME_LEN: usize = 63;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PortForward {
    pub id:         String,
    pub project_id: String,
    pub agent_id:   String,
    pub port:       u16,
    pub route_name: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug)]
pub enum PortForwardError {
    Validation(String),
    DuplicateRouteName,
    NotFound,
    Db,
}

impl std::fmt::Display for PortForwardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortForwardError::Validation(msg) => write!(f, "{msg}"),
            PortForwardError::DuplicateRouteName => write!(f, "a port forward with this route name already exists for this agent"),
            PortForwardError::NotFound => write!(f, "port forward not found"),
            PortForwardError::Db => write!(f, "server error"),
        }
    }
}

impl std::error::Error for PortForwardError {}

fn route_name_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap())
}

pub fn validate_route_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("route_name is required".into());
    }
    if trimmed.len() > MAX_ROUTE_NAME_LEN {
        return Err(format!("route_name must be {MAX_ROUTE_NAME_LEN} characters or fewer"));
    }
    if !route_name_pattern().is_match(trimmed) {
        return Err("route_name may only contain letters, numbers, hyphens, and underscores".into());
    }
    if RESERVED_ROUTE_NAMES.contains(&trimmed.to_lowercase().as_str()) {
        return Err(format!("route_name '{trimmed}' is reserved"));
    }
    Ok(trimmed.to_string())
}

pub fn validate_port(port: u64) -> Result<u16, String> {
    if port < 1 || port > 65535 {
        return Err("port must be between 1 and 65535".into());
    }
    Ok(port as u16)
}

fn row_to_port_forward(row: Value) -> PortForward {
    PortForward {
        id:         row["id"].as_str().unwrap_or("").to_string(),
        project_id: row["project_id"].as_str().unwrap_or("").to_string(),
        agent_id:   row["agent_id"].as_str().unwrap_or("").to_string(),
        port:       row["port"].as_u64().unwrap_or(0) as u16,
        route_name: row["route_name"].as_str().unwrap_or("").to_string(),
        created_at: row["created_at"].as_str().unwrap_or("").to_string(),
        updated_at: row["updated_at"].as_str().unwrap_or("").to_string(),
    }
}

const RETURN_FIELDS: &str = "f.id AS id, f.project_id AS project_id, f.agent_id AS agent_id, \
                              f.port AS port, f.route_name AS route_name, \
                              f.created_at AS created_at, f.updated_at AS updated_at";

pub async fn list_for_agent(
    neo4j:      &Neo4jClient,
    project_id: &str,
    agent_id:   &str,
) -> Result<Vec<PortForward>, PortForwardError> {
    let rows = neo4j.query_read(
        &format!("MATCH (f:PortForward {{agent_id: $aid, project_id: $pid}}) RETURN {RETURN_FIELDS} ORDER BY f.created_at ASC"),
        json!({ "aid": agent_id, "pid": project_id }),
    ).await.map_err(|_| PortForwardError::Db)?;

    Ok(rows.into_iter().map(row_to_port_forward).collect())
}

pub async fn get_by_id(
    neo4j:      &Neo4jClient,
    project_id: &str,
    agent_id:   &str,
    id:         &str,
) -> Result<Option<PortForward>, PortForwardError> {
    let rows = neo4j.query_read(
        &format!("MATCH (f:PortForward {{id: $id, agent_id: $aid, project_id: $pid}}) RETURN {RETURN_FIELDS}"),
        json!({ "id": id, "aid": agent_id, "pid": project_id }),
    ).await.map_err(|_| PortForwardError::Db)?;

    Ok(rows.into_iter().next().map(row_to_port_forward))
}

pub async fn get_by_route(
    neo4j:      &Neo4jClient,
    agent_id:   &str,
    route_name: &str,
) -> Result<Option<PortForward>, PortForwardError> {
    let rows = neo4j.query_read(
        &format!("MATCH (f:PortForward {{agent_id: $aid, route_name: $rn}}) RETURN {RETURN_FIELDS}"),
        json!({ "aid": agent_id, "rn": route_name }),
    ).await.map_err(|_| PortForwardError::Db)?;

    Ok(rows.into_iter().next().map(row_to_port_forward))
}

async fn route_name_taken(
    neo4j:      &Neo4jClient,
    agent_id:   &str,
    route_name: &str,
    excluding_id: Option<&str>,
) -> Result<bool, PortForwardError> {
    let existing = get_by_route(neo4j, agent_id, route_name).await?;
    Ok(match (existing, excluding_id) {
        (Some(f), Some(excl)) => f.id != excl,
        (Some(_), None)       => true,
        (None, _)             => false,
    })
}

pub async fn create(
    neo4j:      &Neo4jClient,
    project_id: &str,
    agent_id:   &str,
    port:       u16,
    route_name: &str,
) -> Result<PortForward, PortForwardError> {
    let route_name = validate_route_name(route_name).map_err(PortForwardError::Validation)?;

    if route_name_taken(neo4j, agent_id, &route_name, None).await? {
        return Err(PortForwardError::DuplicateRouteName);
    }

    let id  = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    neo4j.query_read(
        "CREATE (f:PortForward {
             id: $id, project_id: $pid, agent_id: $aid, port: $port,
             route_name: $rn, created_at: $now, updated_at: $now
         })",
        json!({
            "id": id, "pid": project_id, "aid": agent_id, "port": port as i64,
            "rn": route_name, "now": now,
        }),
    ).await.map_err(|_| PortForwardError::Db)?;

    Ok(PortForward {
        id, project_id: project_id.to_string(), agent_id: agent_id.to_string(),
        port, route_name, created_at: now.clone(), updated_at: now,
    })
}

pub async fn update(
    neo4j:          &Neo4jClient,
    project_id:     &str,
    agent_id:       &str,
    id:             &str,
    new_port:       Option<u16>,
    new_route_name: Option<String>,
) -> Result<PortForward, PortForwardError> {
    let existing = get_by_id(neo4j, project_id, agent_id, id).await?
        .ok_or(PortForwardError::NotFound)?;

    let route_name = match new_route_name {
        Some(name) => validate_route_name(&name).map_err(PortForwardError::Validation)?,
        None       => existing.route_name.clone(),
    };
    let port = new_port.unwrap_or(existing.port);

    if route_name != existing.route_name
        && route_name_taken(neo4j, agent_id, &route_name, Some(id)).await?
    {
        return Err(PortForwardError::DuplicateRouteName);
    }

    let now = Utc::now().to_rfc3339();
    neo4j.query_read(
        "MATCH (f:PortForward {id: $id, agent_id: $aid, project_id: $pid})
         SET f.port = $port, f.route_name = $rn, f.updated_at = $now",
        json!({
            "id": id, "aid": agent_id, "pid": project_id,
            "port": port as i64, "rn": route_name.clone(), "now": now,
        }),
    ).await.map_err(|_| PortForwardError::Db)?;

    Ok(PortForward {
        id: id.to_string(), project_id: project_id.to_string(), agent_id: agent_id.to_string(),
        port, route_name, created_at: existing.created_at, updated_at: now,
    })
}

pub async fn delete(
    neo4j:      &Neo4jClient,
    project_id: &str,
    agent_id:   &str,
    id:         &str,
) -> Result<(), PortForwardError> {
    get_by_id(neo4j, project_id, agent_id, id).await?
        .ok_or(PortForwardError::NotFound)?;

    neo4j.query_read(
        "MATCH (f:PortForward {id: $id, agent_id: $aid, project_id: $pid}) DELETE f",
        json!({ "id": id, "aid": agent_id, "pid": project_id }),
    ).await.map_err(|_| PortForwardError::Db)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_route_name_rejects_empty() {
        assert!(validate_route_name("").is_err());
        assert!(validate_route_name("   ").is_err());
    }

    #[test]
    fn validate_route_name_trims_whitespace() {
        assert_eq!(validate_route_name("  app  ").unwrap(), "app");
    }

    #[test]
    fn validate_route_name_rejects_too_long() {
        let name = "a".repeat(MAX_ROUTE_NAME_LEN + 1);
        assert!(validate_route_name(&name).is_err());
    }

    #[test]
    fn validate_route_name_accepts_max_length() {
        let name = "a".repeat(MAX_ROUTE_NAME_LEN);
        assert!(validate_route_name(&name).is_ok());
    }

    #[test]
    fn validate_route_name_rejects_bad_chars() {
        assert!(validate_route_name("app/sub").is_err());
        assert!(validate_route_name("app?x=1").is_err());
        assert!(validate_route_name("app name").is_err());
    }

    #[test]
    fn validate_route_name_accepts_hyphen_and_underscore() {
        assert!(validate_route_name("my-app_1").is_ok());
    }

    #[test]
    fn validate_route_name_rejects_reserved_names_case_insensitively() {
        assert!(validate_route_name("install.sh").is_err());
        assert!(validate_route_name("INSTALL.SH").is_err());
    }

    #[test]
    fn validate_port_rejects_zero() {
        assert!(validate_port(0).is_err());
    }

    #[test]
    fn validate_port_rejects_too_large() {
        assert!(validate_port(65536).is_err());
    }

    #[test]
    fn validate_port_accepts_boundaries() {
        assert_eq!(validate_port(1).unwrap(), 1);
        assert_eq!(validate_port(65535).unwrap(), 65535);
    }

    #[test]
    fn validate_port_accepts_typical_value() {
        assert_eq!(validate_port(8080).unwrap(), 8080);
    }
}

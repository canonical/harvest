use chrono::{DateTime, Utc};
use dashmap::DashMap;
use harvest_db::Db;
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

use super::client::CollocateHandle;

#[derive(Clone)]
pub struct SessionContainer {
    pub id: String,
    pub name: String,
    pub project_id: String,
    pub conversation_id: String,
    pub created_at: DateTime<Utc>,
    pub persistent: bool,
    pub address: Option<String>,
    pub published: Vec<String>,
}

struct SharedStore {
    db:      Arc<Db>,
    node_id: String,
    handle:  Option<Arc<CollocateHandle>>,
}

#[derive(Default)]
pub struct SessionContainerRegistry {
    containers: DashMap<String, SessionContainer>,
    handles: DashMap<String, Arc<CollocateHandle>>,
    cleanup_lock: Mutex<()>,
    store: OnceLock<SharedStore>,
}

fn container_from_row(row: &Value) -> SessionContainer {
    SessionContainer {
        id: row["container_id"].as_str().unwrap_or_default().to_string(),
        name: row["name"].as_str().unwrap_or_default().to_string(),
        project_id: row["project_id"].as_str().unwrap_or_default().to_string(),
        conversation_id: row["conversation_id"].as_str().unwrap_or_default().to_string(),
        created_at: row["created_at"].as_str()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(Utc::now),
        persistent: row["persistent"].as_bool().unwrap_or(false),
        address: row["address"].as_str().map(str::to_string),
        published: row["published"].as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
    }
}

impl SessionContainerRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn attach_store(&self, db: Arc<Db>, node_id: String, handle: Option<Arc<CollocateHandle>>) {
        let _ = self.store.set(SharedStore { db, node_id, handle });
    }

    fn persist(&self, container: &SessionContainer) {
        let Some(store) = self.store.get() else { return };
        let db = Arc::clone(&store.db);
        let params = json!({
            "id": container.id, "name": container.name, "pid": container.project_id,
            "cid": container.conversation_id, "node": store.node_id, "persistent": container.persistent,
            "address": container.address, "published": container.published,
            "created": container.created_at.to_rfc3339(),
        });
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let result = db.execute(
                    "INSERT INTO collocate_sessions (container_id, name, project_id, conversation_id, node_id, persistent, address, published, created_at)
                     VALUES ($id, $name, $pid, $cid, $node, $persistent, $address, $published, $created::timestamptz)
                     ON CONFLICT (container_id) DO UPDATE SET address = EXCLUDED.address, published = EXCLUDED.published,
                         persistent = EXCLUDED.persistent, node_id = EXCLUDED.node_id",
                    params,
                ).await;
                if let Err(e) = result {
                    tracing::warn!(error = %e, "failed to record collocate session");
                }
            });
        }
    }

    fn forget(&self, container_id: &str) {
        let Some(store) = self.store.get() else { return };
        let db = Arc::clone(&store.db);
        let id = container_id.to_string();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = db.execute("DELETE FROM collocate_sessions WHERE container_id = $id", json!({ "id": id })).await;
            });
        }
    }

    pub fn register(&self, container: SessionContainer) {
        self.persist(&container);
        self.containers.insert(container.id.clone(), container);
    }

    pub fn register_with_handle(
        &self,
        container: SessionContainer,
        handle: Arc<CollocateHandle>,
    ) {
        self.persist(&container);
        self.handles.insert(container.id.clone(), handle);
        self.containers.insert(container.id.clone(), container);
    }

    async fn shared_rows(&self, sql: &str, params: Value) -> Option<Vec<SessionContainer>> {
        let store = self.store.get()?;
        let rows = store.db.query(sql, params).await.ok()?;
        Some(rows.iter().map(container_from_row).collect())
    }

    pub async fn verify_ownership_shared(
        &self,
        container_id: &str,
        project_id: &str,
        conversation_id: &str,
    ) -> Option<SessionContainer> {
        if let Some(local) = self.verify_ownership(container_id, project_id, conversation_id) {
            return Some(local);
        }
        let found = self.shared_rows(
            "SELECT container_id, name, project_id, conversation_id, persistent, address, published, created_at
             FROM collocate_sessions WHERE container_id = $id AND project_id = $pid AND conversation_id = $cid",
            json!({ "id": container_id, "pid": project_id, "cid": conversation_id }),
        ).await?;
        found.into_iter().next()
    }

    pub async fn list_for_session_shared(&self, project_id: &str, conversation_id: &str) -> Vec<SessionContainer> {
        let mut containers = self.list_for_session(project_id, conversation_id);
        let shared = self.shared_rows(
            "SELECT container_id, name, project_id, conversation_id, persistent, address, published, created_at
             FROM collocate_sessions WHERE project_id = $pid AND conversation_id = $cid ORDER BY created_at",
            json!({ "pid": project_id, "cid": conversation_id }),
        ).await.unwrap_or_default();
        for container in shared {
            if !containers.iter().any(|c| c.id == container.id) {
                containers.push(container);
            }
        }
        containers
    }

    pub async fn count_for_project_shared(&self, project_id: &str) -> usize {
        let Some(store) = self.store.get() else { return self.count_for_project(project_id) };
        let rows = store.db.query(
            "SELECT count(*) AS n FROM collocate_sessions WHERE project_id = $pid",
            json!({ "pid": project_id }),
        ).await;
        match rows {
            Ok(rows) => rows.first().and_then(|r| r["n"].as_i64()).unwrap_or(0) as usize,
            Err(_) => self.count_for_project(project_id),
        }
    }

    pub async fn reap_dead_nodes(&self, timeout_secs: f64) -> usize {
        let Some(store) = self.store.get() else { return 0 };
        let rows = store.db.query(
            "DELETE FROM collocate_sessions s
             WHERE s.persistent = false AND s.node_id <> $node AND NOT EXISTS (
                 SELECT 1 FROM cluster_nodes n
                 WHERE n.node_id = s.node_id AND n.heartbeat_at > now() - make_interval(secs => $timeout::float8))
             RETURNING container_id",
            json!({ "node": store.node_id, "timeout": timeout_secs }),
        ).await.unwrap_or_default();
        if let Some(handle) = &store.handle {
            for row in &rows {
                if let Some(id) = row["container_id"].as_str() {
                    let _ = handle.rm_force(id).await;
                }
            }
        }
        rows.len()
    }

    pub fn get(&self, container_id: &str) -> Option<SessionContainer> {
        self.containers.get(container_id).map(|e| e.clone())
    }

    pub fn verify_ownership(
        &self,
        container_id: &str,
        project_id: &str,
        conversation_id: &str,
    ) -> Option<SessionContainer> {
        self.containers.get(container_id).and_then(|e| {
            let c = e.value();
            if c.project_id == project_id && c.conversation_id == conversation_id {
                Some(c.clone())
            } else {
                None
            }
        })
    }

    pub fn list_for_session(&self, project_id: &str, conversation_id: &str) -> Vec<SessionContainer> {
        self.containers
            .iter()
            .filter(|e| {
                let c = e.value();
                c.project_id == project_id && c.conversation_id == conversation_id
            })
            .map(|e| e.clone())
            .collect()
    }

    pub fn list_all(&self) -> Vec<SessionContainer> {
        self.containers.iter().map(|e| e.clone()).collect()
    }

    pub fn count_for_project(&self, project_id: &str) -> usize {
        self.containers
            .iter()
            .filter(|e| e.value().project_id == project_id)
            .count()
    }

    pub fn remove_entry(&self, container_id: &str) -> Option<SessionContainer> {
        self.forget(container_id);
        self.handles.remove(container_id);
        self.containers.remove(container_id).map(|(_, c)| c)
    }

    pub async fn cleanup_session(&self, project_id: &str, conversation_id: &str) -> Vec<String> {
        let _lock = self.cleanup_lock.lock().await;
        let to_remove: Vec<SessionContainer> = self
            .containers
            .iter()
            .filter(|e| {
                let c = e.value();
                c.project_id == project_id
                    && c.conversation_id == conversation_id
                    && !c.persistent
            })
            .map(|e| e.clone())
            .collect();

        let mut removed = Vec::new();
        for c in &to_remove {
            if let Some(handle) = self.handles.get(&c.id) {
                let _ = handle.rm_force(&c.id).await;
            }
            self.handles.remove(&c.id);
            self.containers.remove(&c.id);
            self.forget(&c.id);
            removed.push(c.id.clone());
        }
        removed
    }

    pub async fn cleanup_container(&self, container_id: &str) -> Option<SessionContainer> {
        if let Some(handle) = self.handles.get(container_id) {
            let _ = handle.rm_force(container_id).await;
        }
        self.handles.remove(container_id);
        self.forget(container_id);
        self.containers.remove(container_id).map(|(_, c)| c)
    }
}

pub struct SessionGuard {
    registry: Arc<SessionContainerRegistry>,
    project_id: String,
    conversation_id: String,
    dropped: bool,
}

impl SessionGuard {
    pub fn new(
        registry: Arc<SessionContainerRegistry>,
        project_id: String,
        conversation_id: String,
    ) -> Self {
        Self {
            registry,
            project_id,
            conversation_id,
            dropped: false,
        }
    }

    pub fn dismiss(&mut self) {
        self.dropped = true;
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        if self.dropped {
            return;
        }
        let registry = Arc::clone(&self.registry);
        let project_id = self.project_id.clone();
        let conversation_id = self.conversation_id.clone();
        tokio::spawn(async move {
            let removed = registry.cleanup_session(&project_id, &conversation_id).await;
            if !removed.is_empty() {
                tracing::info!(
                    removed = removed.len(),
                    containers = ?removed,
                    "auto-cleaned session containers"
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_container(id: &str, project: &str, conv: &str, persistent: bool) -> SessionContainer {
        SessionContainer {
            id: id.into(),
            name: format!("test-{id}"),
            project_id: project.into(),
            conversation_id: conv.into(),
            created_at: Utc::now(),
            persistent,
            address: Some("172.30.0.2".into()),
            published: vec![],
        }
    }

    #[test]
    fn register_and_get() {
        let reg = SessionContainerRegistry::new();
        let c = make_container("c1", "p1", "conv1", false);
        reg.register(c.clone());
        let got = reg.get("c1").unwrap();
        assert_eq!(got.name, "test-c1");
        assert_eq!(got.project_id, "p1");
    }

    #[test]
    fn verify_ownership_rejects_mismatched_project() {
        let reg = SessionContainerRegistry::new();
        reg.register(make_container("c1", "p1", "conv1", false));
        assert!(reg.verify_ownership("c1", "p1", "conv1").is_some());
        assert!(reg.verify_ownership("c1", "p2", "conv1").is_none());
        assert!(reg.verify_ownership("c1", "p1", "conv2").is_none());
    }

    #[test]
    fn list_for_session_filters_correctly() {
        let reg = SessionContainerRegistry::new();
        reg.register(make_container("c1", "p1", "conv1", false));
        reg.register(make_container("c2", "p1", "conv1", true));
        reg.register(make_container("c3", "p1", "conv2", false));
        reg.register(make_container("c4", "p2", "conv1", false));
        let list = reg.list_for_session("p1", "conv1");
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|c| c.id == "c1"));
        assert!(list.iter().any(|c| c.id == "c2"));
    }

    #[test]
    fn count_for_project() {
        let reg = SessionContainerRegistry::new();
        reg.register(make_container("c1", "p1", "conv1", false));
        reg.register(make_container("c2", "p1", "conv2", false));
        reg.register(make_container("c3", "p2", "conv1", false));
        assert_eq!(reg.count_for_project("p1"), 2);
        assert_eq!(reg.count_for_project("p2"), 1);
        assert_eq!(reg.count_for_project("p3"), 0);
    }

    #[test]
    fn remove_entry_removes_container() {
        let reg = SessionContainerRegistry::new();
        reg.register(make_container("c1", "p1", "conv1", false));
        assert!(reg.get("c1").is_some());
        let removed = reg.remove_entry("c1");
        assert!(removed.is_some());
        assert!(reg.get("c1").is_none());
    }

    #[tokio::test]
    async fn cleanup_session_removes_non_persistent_only() {
        let reg = SessionContainerRegistry::new();
        reg.register(make_container("c1", "p1", "conv1", false));
        reg.register(make_container("c2", "p1", "conv1", true));
        reg.register(make_container("c3", "p1", "conv2", false));
        let removed = reg.cleanup_session("p1", "conv1").await;
        assert_eq!(removed.len(), 1);
        assert!(removed.contains(&"c1".to_string()));
        assert!(reg.get("c1").is_none());
        assert!(reg.get("c2").is_some());
        assert!(reg.get("c3").is_some());
    }

    #[tokio::test]
    async fn session_guard_does_not_clean_when_dismissed() {
        let reg = SessionContainerRegistry::new();
        reg.register(make_container("c1", "p1", "conv1", false));
        {
            let mut guard = SessionGuard::new(Arc::clone(&reg), "p1".into(), "conv1".into());
            guard.dismiss();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(reg.get("c1").is_some());
    }

    #[tokio::test]
    async fn cleanup_container_removes_specific_container() {
        let reg = SessionContainerRegistry::new();
        reg.register(make_container("c1", "p1", "conv1", false));
        let removed = reg.cleanup_container("c1").await;
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, "c1");
        assert!(reg.get("c1").is_none());
    }
}

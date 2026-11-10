use std::sync::Arc;
use std::time::{Duration, Instant};

use harvest_db::Db;
use harvest_search::Scope;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::cluster::bus::ClusterBus;
use crate::cluster::Shutdown;
use crate::metrics::{SEARCH_INDEX_LOAD, SEARCH_INDEX_LOADED, SEARCH_INDEX_STALENESS};
use crate::search::SearchService;

const RECENT_SCOPES_SQL: &str = r#"
SELECT repo, version, max(created_at) AS last_used
FROM search_traces
WHERE repo IS NOT NULL AND version IS NOT NULL AND created_at > now() - interval '7 days'
GROUP BY repo, version
ORDER BY max(created_at) DESC
LIMIT 20
"#;

pub async fn hot_scopes(service: &SearchService, db: &Db) -> Vec<Scope> {
    let mut scopes: Vec<Scope> = service
        .config
        .warm_scopes()
        .into_iter()
        .map(|(repo, version)| Scope::new(repo, version))
        .collect();
    if let Ok(rows) = db.query(RECENT_SCOPES_SQL, json!({})).await {
        for row in rows {
            let scope = Scope::new(
                row["repo"].as_str().unwrap_or_default(),
                row["version"].as_str().unwrap_or_default(),
            );
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
    }
    scopes
}

async fn load(service: &SearchService, scope: &Scope) {
    let started = Instant::now();
    match service.registry.ensure(scope).await {
        Ok(Some(_)) => {
            SEARCH_INDEX_LOAD.with(&[]).observe(started.elapsed());
            SEARCH_INDEX_LOADED.set(&[("repo", &scope.repo), ("version", &scope.version)], 1);
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, ?scope, "warming the search index failed"),
    }
}

async fn refresh(service: &SearchService, repo_prefix: Option<&str>, since: Instant) {
    for (scope, _) in service.registry.loaded() {
        if repo_prefix.is_some_and(|prefix| !format!("{}:", scope.repo).starts_with(prefix)) {
            continue;
        }
        match service.registry.refresh(&scope).await {
            Ok(true) => SEARCH_INDEX_STALENESS.with(&[]).observe(since.elapsed()),
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, ?scope, "refreshing the search index failed"),
        }
    }
}

pub fn spawn(service: Arc<SearchService>, db: Arc<Db>, bus: Arc<ClusterBus>, shutdown: Shutdown) {
    let mut events = bus.subscribe(crate::api::repositories::GRAPH_TOPIC);
    tokio::spawn(async move {
        for scope in hot_scopes(&service, &db).await {
            load(&service, &scope).await;
        }
        let mut ticker =
            tokio::time::interval(Duration::from_secs(service.config.refresh_secs.max(1)));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let stop = shutdown.wait();
        tokio::pin!(stop);
        loop {
            tokio::select! {
                _ = &mut stop => return,
                event = events.recv() => match event {
                    Ok(prefix) => refresh(&service, Some(&prefix), Instant::now()).await,
                    Err(RecvError::Lagged(_)) => refresh(&service, None, Instant::now()).await,
                    Err(RecvError::Closed) => return,
                },
                _ = ticker.tick() => refresh(&service, None, Instant::now()).await,
            }
        }
    });
}

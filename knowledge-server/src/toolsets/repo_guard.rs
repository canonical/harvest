use async_trait::async_trait;
use serde_json::Value;

use crate::access::repos::RepoScope;
use crate::agent::tool::{Tool, ToolOutput};
use crate::llm::types::ToolDefinition;

const REPO_PARAMS: &[&str] = &["repo", "repository"];
const UNRESTRICTED_ONLY: &[&str] = &["run_sql"];

pub struct RepoGuard {
    inner: Box<dyn Tool>,
    scope: RepoScope,
}

impl RepoGuard {
    pub fn wrap(tools: Vec<Box<dyn Tool>>, scope: &RepoScope) -> Vec<Box<dyn Tool>> {
        tools
            .into_iter()
            .filter(|tool| {
                scope.is_all() || !UNRESTRICTED_ONLY.contains(&tool.definition().name.as_str())
            })
            .map(|inner| {
                Box::new(Self {
                    inner,
                    scope: scope.clone(),
                }) as Box<dyn Tool>
            })
            .collect()
    }

    fn outside_scope(&self, params: &Value) -> Option<String> {
        REPO_PARAMS
            .iter()
            .filter_map(|key| params[*key].as_str())
            .map(str::trim)
            .find(|repo| !repo.is_empty() && !self.scope.allows(repo))
            .map(String::from)
    }

    fn scoped(&self, mut params: Value) -> Result<Value, String> {
        if let Some(repo) = self.outside_scope(&params) {
            return Err(format!("error: repository '{repo}' not found"));
        }
        self.scope.write_into(&mut params);
        Ok(params)
    }
}

#[async_trait]
impl Tool for RepoGuard {
    fn definition(&self) -> ToolDefinition {
        self.inner.definition()
    }

    async fn execute(&self, params: Value) -> anyhow::Result<String> {
        match self.scoped(params) {
            Ok(params) => self.inner.execute(params).await,
            Err(refusal) => Ok(refusal),
        }
    }

    async fn run(&self, params: Value) -> anyhow::Result<ToolOutput> {
        match self.scoped(params) {
            Ok(params) => self.inner.run(params).await,
            Err(refusal) => Ok(ToolOutput::text(refusal)),
        }
    }

    fn preview(&self, result: &str) -> String {
        self.inner.preview(result)
    }

    fn requires_confirmation(&self) -> bool {
        self.inner.requires_confirmation()
    }

    fn reads_code(&self) -> bool {
        self.inner.reads_code()
    }

    fn read_budget(&self) -> Option<usize> {
        self.inner.read_budget()
    }

    async fn prefetch(&self, question: &str, earlier: &str) -> Option<(Value, ToolOutput)> {
        let (params, output) = self.inner.prefetch(question, earlier).await?;
        match self.outside_scope(&params) {
            Some(_) => None,
            None => Some((params, output)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::repos::RepoScope;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    struct Recorder {
        name: &'static str,
        seen: Arc<Mutex<Option<Value>>>,
    }

    #[async_trait]
    impl Tool for Recorder {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.name.into(),
                description: String::new(),
                parameters: json!({}),
            }
        }

        async fn execute(&self, params: Value) -> anyhow::Result<String> {
            *self.seen.lock().unwrap() = Some(params);
            Ok("ran".into())
        }
    }

    fn guarded(
        name: &'static str,
        scope: &RepoScope,
    ) -> (Vec<Box<dyn Tool>>, Arc<Mutex<Option<Value>>>) {
        let seen = Arc::new(Mutex::new(None));
        let tools = RepoGuard::wrap(
            vec![Box::new(Recorder {
                name,
                seen: Arc::clone(&seen),
            })],
            scope,
        );
        (tools, seen)
    }

    #[tokio::test]
    async fn a_repository_outside_the_scope_is_reported_as_missing() {
        let (tools, seen) = guarded("read_sources", &RepoScope::only(["cinder"]));
        let out = tools[0].execute(json!({ "repo": "secret" })).await.unwrap();
        assert_eq!(out, "error: repository 'secret' not found");
        assert!(seen.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn the_scope_reaches_the_tool_and_cannot_be_widened_by_the_caller() {
        let (tools, seen) = guarded("search_symbols", &RepoScope::only(["cinder"]));
        tools[0]
            .execute(json!({ "query": "x", "_scope_all": true }))
            .await
            .unwrap();
        let params = seen.lock().unwrap().clone().unwrap();
        assert_eq!(
            RepoScope::read_from(&params),
            (false, vec!["cinder".to_string()])
        );
    }

    #[test]
    fn raw_sql_is_only_offered_to_unrestricted_callers() {
        assert!(guarded("run_sql", &RepoScope::only(["cinder"]))
            .0
            .is_empty());
        assert_eq!(guarded("run_sql", &RepoScope::All).0.len(), 1);
    }
}

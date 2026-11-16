use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use super::{freshness, index, CodeEnv};
use crate::agent::tool::{Tool, ToolOutput};
use crate::llm::types::ToolDefinition;

pub struct WorkspaceOverlay {
    inner: Box<dyn Tool>,
    env: CodeEnv,
    session_id: String,
}

impl WorkspaceOverlay {
    pub fn wrap(tools: Vec<Box<dyn Tool>>, env: &CodeEnv, session_id: &str) -> Vec<Box<dyn Tool>> {
        tools
            .into_iter()
            .map(|inner| {
                Box::new(Self {
                    inner,
                    env: env.clone(),
                    session_id: session_id.to_string(),
                }) as Box<dyn Tool>
            })
            .collect()
    }

    async fn annotate(&self, params: &Value, text: String) -> String {
        let Some(repo) = params["repo"].as_str().filter(|_| !text.starts_with("error:")) else {
            return text;
        };
        match self.footer(repo, params["version"].as_str(), &text).await {
            Ok(Some(footer)) => format!("{text}\n\n{footer}"),
            Ok(None) => text,
            Err(e) => format!("{text}\n\n(Could not compare these results with your workspace: {e})"),
        }
    }

    async fn footer(&self, repo: &str, version: Option<&str>, text: &str) -> Result<Option<String>> {
        let Some(indexed) = index::of_session(&self.env, &self.session_id).await? else {
            return Ok(None);
        };
        if !indexed.describes(repo, version) {
            return Ok(None);
        }
        let drift = freshness::drift(&self.env, &self.session_id, &indexed).await?;
        Ok(drift.footer(text, &indexed.repo))
    }
}

#[async_trait]
impl Tool for WorkspaceOverlay {
    fn definition(&self) -> ToolDefinition {
        self.inner.definition()
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let text = self.inner.execute(params.clone()).await?;
        Ok(self.annotate(&params, text).await)
    }

    async fn run(&self, params: Value) -> Result<ToolOutput> {
        let mut output = self.inner.run(params.clone()).await?;
        output.text = self.annotate(&params, output.text).await;
        Ok(output)
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
        let (params, mut output) = self.inner.prefetch(question, earlier).await?;
        output.text = self.annotate(&params, output.text).await;
        Some((params, output))
    }
}

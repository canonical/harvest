use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use super::store;
use crate::agent::tool::{Tool, ToolOutput};
use crate::code::job_tools::REPORT_CHARS;
use crate::code::CodeEnv;
use crate::llm::types::ToolDefinition;

pub struct JobReports {
    inner: Box<dyn Tool>,
    env: CodeEnv,
    session_id: String,
    chat_id: String,
}

impl JobReports {
    pub fn wrap(tools: Vec<Box<dyn Tool>>, env: &CodeEnv, session_id: &str, chat_id: &str) -> Vec<Box<dyn Tool>> {
        tools
            .into_iter()
            .map(|inner| {
                Box::new(Self {
                    inner,
                    env: env.clone(),
                    session_id: session_id.to_string(),
                    chat_id: chat_id.to_string(),
                }) as Box<dyn Tool>
            })
            .collect()
    }

    async fn finished_jobs(&self) -> String {
        match store::take_unreported(&self.env, &self.session_id, &self.chat_id).await {
            Ok(jobs) => jobs.iter().map(|job| format!("\n\n{}", job.report(REPORT_CHARS))).collect(),
            Err(e) => {
                tracing::warn!(session = self.session_id, error = %e, "could not read finished jobs");
                String::new()
            }
        }
    }
}

#[async_trait]
impl Tool for JobReports {
    fn definition(&self) -> ToolDefinition {
        self.inner.definition()
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let text = self.inner.execute(params).await?;
        Ok(format!("{text}{}", self.finished_jobs().await))
    }

    async fn run(&self, params: Value) -> Result<ToolOutput> {
        let mut output = self.inner.run(params).await?;
        output.text.push_str(&self.finished_jobs().await);
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
        self.inner.prefetch(question, earlier).await
    }
}

use std::time::Duration;

use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

use super::jobs::{shell, store, Job};
use super::CodeEnv;
use crate::agent::tool::Tool;
use crate::llm::types::ToolDefinition;

pub const REPORT_CHARS: usize = 4_000;
const DEFAULT_WAIT_SECS: u64 = 120;
const MAX_WAIT_SECS: u64 = 600;

#[derive(Clone, Copy)]
enum Operation {
    Output,
    Wait,
}

pub struct JobTool {
    env: CodeEnv,
    session_id: String,
    operation: Operation,
}

impl JobTool {
    pub fn all(env: &CodeEnv, session_id: &str) -> Vec<Box<dyn Tool>> {
        [Operation::Output, Operation::Wait]
            .into_iter()
            .map(|operation| {
                Box::new(Self {
                    env: env.clone(),
                    session_id: session_id.to_string(),
                    operation,
                }) as Box<dyn Tool>
            })
            .collect()
    }

    async fn job(&self, params: &Value) -> Result<Job> {
        let id = params["job_id"].as_str().unwrap_or_default();
        let job = match self.operation {
            Operation::Output => shell::refresh(&self.env, id).await?,
            Operation::Wait => {
                let secs = params["timeout_seconds"].as_u64().unwrap_or(DEFAULT_WAIT_SECS).clamp(1, MAX_WAIT_SECS);
                shell::wait(&self.env, id, Duration::from_secs(secs)).await?
            }
        };
        if job.session_id != self.session_id {
            bail!("this job belongs to another session");
        }
        if !job.is_running() {
            store::mark_reported(&self.env, &job.id).await?;
        }
        Ok(job)
    }
}

#[async_trait]
impl Tool for JobTool {
    fn definition(&self) -> ToolDefinition {
        let job_id = json!({ "type": "string", "description": "The id run_command returned for a background command" });
        match self.operation {
            Operation::Output => ToolDefinition {
                name: "job_output".into(),
                description: "Read the status and latest output of a background job.".into(),
                parameters: json!({ "type": "object", "properties": { "job_id": job_id }, "required": ["job_id"] }),
            },
            Operation::Wait => ToolDefinition {
                name: "wait_job".into(),
                description: "Wait until a background job finishes, or until the timeout, then report its status and output. Do other work first when you can.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "job_id": job_id,
                        "timeout_seconds": { "type": "integer", "description": "Defaults to 120, at most 600" }
                    },
                    "required": ["job_id"]
                }),
            },
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        match self.job(&params).await {
            Ok(job) => Ok(job.report(REPORT_CHARS)),
            Err(e) => Ok(format!("error: {e}")),
        }
    }
}

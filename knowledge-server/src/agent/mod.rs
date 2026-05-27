pub mod graph_tools;
pub mod prompt;
pub mod tool;

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::llm::{
    types::{ContentPart, LlmResponse, Message, MessageContent, ToolCall, ToolDefinition},
    LlmProvider,
};
use tool::Tool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub repo: String,
    pub version: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Serialize)]
pub struct QueryResponse {
    pub answer: String,
    pub sources: Vec<Source>,
    pub tool_calls_made: usize,
}

pub struct Agent {
    llm: Arc<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    max_iterations: usize,
}

impl Agent {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        tools: Vec<Box<dyn Tool>>,
        max_iterations: usize,
    ) -> Self {
        Self { llm, tools, max_iterations }
    }

    pub async fn query(&self, user_query: &str) -> Result<QueryResponse> {
        let tool_defs: Vec<ToolDefinition> =
            self.tools.iter().map(|t| t.definition()).collect();

        let tool_map: HashMap<String, &dyn Tool> =
            self.tools.iter().map(|t| (t.definition().name, t.as_ref())).collect();

        let mut messages = vec![
            Message::system(prompt::system_prompt()),
            Message::user(user_query),
        ];

        let mut iterations = 0;

        let final_text = loop {
            if iterations >= self.max_iterations {
                tracing::warn!("agent hit max_iterations={} — returning partial answer", self.max_iterations);
                break self.last_assistant_text(&messages);
            }

            match self.llm.chat(&messages, &tool_defs).await? {
                LlmResponse::Message { text } => break text,

                LlmResponse::ToolCalls(calls) => {
                    iterations += 1;

                    let call_parts: Vec<ContentPart> = calls
                        .iter()
                        .map(|c| ContentPart::ToolUse {
                            id: c.id.clone(),
                            name: c.name.clone(),
                            input: c.input.clone(),
                        })
                        .collect();
                    messages.push(Message {
                        role: crate::llm::types::Role::Assistant,
                        content: MessageContent::Parts(call_parts),
                    });

                    for call in &calls {
                        let result = self.execute_tool_call(call, &tool_map).await;
                        messages.push(Message {
                            role: crate::llm::types::Role::User,
                            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                                tool_use_id: call.id.clone(),
                                content: result,
                                is_error: false,
                            }]),
                        });
                    }
                }
            }
        };

        let sources = parse_citations(&final_text);
        Ok(QueryResponse {
            answer: final_text,
            sources,
            tool_calls_made: iterations,
        })
    }

    async fn execute_tool_call(
        &self,
        call: &ToolCall,
        tool_map: &HashMap<String, &dyn Tool>,
    ) -> String {
        tracing::info!(tool = call.name, "executing tool call");
        match tool_map.get(call.name.as_str()) {
            None => format!("error: unknown tool '{}'", call.name),
            Some(tool) => match tool.execute(call.input.clone()).await {
                Ok(output) => output,
                Err(e) => {
                    tracing::error!(tool = call.name, error = %e, "tool execution failed");
                    format!("error: {e}")
                }
            },
        }
    }

    fn last_assistant_text(&self, messages: &[Message]) -> String {
        messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, crate::llm::types::Role::Assistant))
            .and_then(|m| match &m.content {
                MessageContent::Text(t) => Some(t.clone()),
                MessageContent::Parts(parts) => parts.iter().find_map(|p| match p {
                    ContentPart::Text { text } => Some(text.clone()),
                    _ => None,
                }),
            })
            .unwrap_or_default()
    }
}

fn parse_citations(text: &str) -> Vec<Source> {
    let re = Regex::new(r"\[([^:\]\s]+):([^:\]\s]+):([^:\]\s]+):(\d+)\]").unwrap();
    let mut seen = HashSet::new();
    let mut sources = Vec::new();

    for cap in re.captures_iter(text) {
        let key = cap[0].to_string();
        if seen.insert(key) {
            sources.push(Source {
                repo:    cap[1].to_string(),
                version: cap[2].to_string(),
                file:    cap[3].to_string(),
                line:    cap[4].parse().unwrap_or(0),
            });
        }
    }
    sources
}

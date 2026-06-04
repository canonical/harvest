pub mod graph_tools;
pub mod prompt;
pub mod tool;

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::llm::{
    types::{ContentPart, LlmResponse, Message, MessageContent, ToolCall, ToolDefinition},
    LlmProvider,
};
use tool::Tool;

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryMessage {
    pub role: String,
    pub text: String,
}

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

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    ToolCall { name: String, input: serde_json::Value },
    ToolResult { name: String, preview: String },
    Done { answer: String, sources: Vec<Source>, tool_calls_made: usize },
    Error { message: String },
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

    pub async fn query(&self, user_query: &str, history: &[HistoryMessage]) -> Result<QueryResponse> {
        let tool_defs: Vec<ToolDefinition> =
            self.tools.iter().map(|t| t.definition()).collect();

        let tool_map: HashMap<String, &dyn Tool> =
            self.tools.iter().map(|t| (t.definition().name, t.as_ref())).collect();

        let mut messages = vec![Message::system(prompt::system_prompt())];
        messages.extend(history_to_messages(history));
        messages.push(Message::user(user_query));

        let mut iterations = 0;

        let final_text = loop {
            if iterations >= self.max_iterations {
                tracing::warn!("agent hit max_iterations={} — requesting synthesis", self.max_iterations);
                messages.push(Message::user(
                    "You have used the maximum number of tool calls. \
                     Synthesize what you have gathered so far into a final answer.",
                ));
                match self.llm.chat(&messages, &[]).await {
                    Ok(LlmResponse::Message { text }) => break text,
                    Ok(LlmResponse::ToolCalls(_)) | Err(_) => break self.last_assistant_text(&messages),
                }
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

    pub async fn describe_tool_call(&self, name: &str, input: &serde_json::Value) -> String {
        let prompt = format!(
            "In 8 words or fewer, describe what this specific tool call is doing. \
             Be concrete and mention key input values.\n\
             Tool: {name}\nInput: {input}\n\
             Reply with only the description, no trailing punctuation.",
        );
        match self.llm.chat(&[Message::user(prompt)], &[]).await {
            Ok(LlmResponse::Message { text }) => text.trim().to_string(),
            _ => name.to_string(),
        }
    }

    pub async fn query_streaming(&self, user_query: &str, history: &[HistoryMessage], tx: mpsc::Sender<AgentEvent>) {
        let tool_defs: Vec<ToolDefinition> =
            self.tools.iter().map(|t| t.definition()).collect();

        let tool_map: HashMap<String, &dyn Tool> =
            self.tools.iter().map(|t| (t.definition().name, t.as_ref())).collect();

        let mut messages = vec![Message::system(prompt::system_prompt())];
        messages.extend(history_to_messages(history));
        messages.push(Message::user(user_query));

        let mut iterations = 0;

        let final_text = loop {
            if iterations >= self.max_iterations {
                tracing::warn!("agent hit max_iterations={} — requesting synthesis", self.max_iterations);
                messages.push(Message::user(
                    "You have used the maximum number of tool calls. \
                     Synthesize what you have gathered so far into a final answer.",
                ));
                match self.llm.chat(&messages, &[]).await {
                    Ok(LlmResponse::Message { text }) => break text,
                    Ok(LlmResponse::ToolCalls(_)) | Err(_) => break self.last_assistant_text(&messages),
                }
            }

            let response = match self.llm.chat(&messages, &tool_defs).await {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(AgentEvent::Error { message: e.to_string() }).await;
                    return;
                }
            };

            match response {
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
                        let _ = tx.send(AgentEvent::ToolCall {
                            name: call.name.clone(),
                            input: call.input.clone(),
                        }).await;

                        let result = self.execute_tool_call(call, &tool_map).await;

                        let preview = tool_map.get(call.name.as_str())
                            .map(|t| t.preview(&result))
                            .unwrap_or_else(|| result.chars().take(3000).collect());
                        let _ = tx.send(AgentEvent::ToolResult {
                            name: call.name.clone(),
                            preview,
                        }).await;

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
        let _ = tx.send(AgentEvent::Done {
            answer: final_text,
            sources,
            tool_calls_made: iterations,
        }).await;
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

fn history_to_messages(history: &[HistoryMessage]) -> Vec<Message> {
    history.iter().map(|h| {
        if h.role == "assistant" {
            Message::assistant_text(&h.text)
        } else {
            Message::user(&h.text)
        }
    }).collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct MockLlm {
        responses: Mutex<VecDeque<LlmResponse>>,
    }

    impl MockLlm {
        fn new(responses: Vec<LlmResponse>) -> Arc<Self> {
            Arc::new(Self { responses: Mutex::new(responses.into()) })
        }
    }

    #[async_trait]
    impl LlmProvider for MockLlm {
        async fn chat(&self, _messages: &[Message], _tools: &[ToolDefinition]) -> Result<LlmResponse> {
            self.responses.lock().unwrap().pop_front()
                .ok_or_else(|| anyhow::anyhow!("MockLlm: no more responses"))
        }
    }

    struct MockTool {
        name: String,
        returns: String,
    }

    impl MockTool {
        fn new(name: &str, returns: &str) -> Box<Self> {
            Box::new(Self { name: name.into(), returns: returns.into() })
        }
    }

    #[async_trait]
    impl Tool for MockTool {
        fn definition(&self) -> crate::llm::types::ToolDefinition {
            crate::llm::types::ToolDefinition {
                name: self.name.clone(),
                description: String::new(),
                parameters: serde_json::json!({}),
            }
        }
        async fn execute(&self, _params: serde_json::Value) -> Result<String> {
            Ok(self.returns.clone())
        }
    }

    fn tool_call(name: &str) -> LlmResponse {
        LlmResponse::ToolCalls(vec![ToolCall {
            id: "tc_1".into(),
            name: name.into(),
            input: serde_json::json!({}),
        }])
    }

    fn text(s: &str) -> LlmResponse {
        LlmResponse::Message { text: s.into() }
    }

    fn agent_with(llm: Arc<dyn LlmProvider>, tools: Vec<Box<dyn Tool>>, max: usize) -> Agent {
        Agent::new(llm, tools, max)
    }

    #[tokio::test]
    async fn text_on_first_turn_returns_immediately() {
        let llm = MockLlm::new(vec![text("all done")]);
        let agent = agent_with(llm, vec![], 5);
        let resp = agent.query("hi", &[]).await.unwrap();
        assert_eq!(resp.answer, "all done");
        assert_eq!(resp.tool_calls_made, 0);
    }

    #[tokio::test]
    async fn one_tool_call_then_text_counts_one_iteration() {
        let llm = MockLlm::new(vec![
            tool_call("my_tool"),
            text("result arrived"),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5);
        let resp = agent.query("hi", &[]).await.unwrap();
        assert_eq!(resp.answer, "result arrived");
        assert_eq!(resp.tool_calls_made, 1);
    }

    #[tokio::test]
    async fn two_tool_call_turns_count_two_iterations() {
        let llm = MockLlm::new(vec![
            tool_call("my_tool"),
            tool_call("my_tool"),
            text("done after two rounds"),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5);
        let resp = agent.query("hi", &[]).await.unwrap();
        assert_eq!(resp.tool_calls_made, 2);
    }

    #[tokio::test]
    async fn max_iterations_returns_last_assistant_text() {
        let llm = MockLlm::new(vec![
            text("partial answer so far"),
            tool_call("my_tool"),
        ]);
        let agent = agent_with(
            MockLlm::new(vec![text("partial answer so far")]),
            vec![],
            0,
        );
        let resp = agent.query("hi", &[]).await.unwrap();
        assert_eq!(resp.tool_calls_made, 0);
    }

    #[tokio::test]
    async fn unknown_tool_name_produces_error_string_not_panic() {
        let llm = MockLlm::new(vec![
            tool_call("nonexistent_tool"),
            text("handled gracefully"),
        ]);
        let agent = agent_with(llm, vec![], 5);
        let resp = agent.query("hi", &[]).await.unwrap();
        assert_eq!(resp.answer, "handled gracefully");
    }

    #[test]
    fn single_citation_parsed() {
        let sources = parse_citations("see [myrepo:v1.0:src/lib.rs:42]");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].repo, "myrepo");
        assert_eq!(sources[0].version, "v1.0");
        assert_eq!(sources[0].file, "src/lib.rs");
        assert_eq!(sources[0].line, 42);
    }

    #[test]
    fn multiple_citations_parsed() {
        let sources = parse_citations(
            "from [repo:v1:a.rs:1] and also [repo:v2:b.rs:99]"
        );
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn duplicate_citations_deduplicated() {
        let sources = parse_citations(
            "[r:v1:f.rs:1] mentioned twice [r:v1:f.rs:1]"
        );
        assert_eq!(sources.len(), 1);
    }

    #[test]
    fn malformed_citation_ignored() {
        let sources = parse_citations("bad [only:two:fields] here");
        assert!(sources.is_empty());
    }

    #[test]
    fn no_citations_returns_empty() {
        let sources = parse_citations("plain text with no brackets at all");
        assert!(sources.is_empty());
    }

    #[test]
    fn citation_line_zero_on_invalid_number() {
        let sources = parse_citations("[r:v1:f.rs:0]");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].line, 0);
    }
}

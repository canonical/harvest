pub mod graph_tools;
pub mod machine_tools;
pub mod skill_tools;
pub mod prompt;
pub mod tool;

use anyhow::Result;
use futures::future::join_all;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::llm::{
    types::{ContentPart, LlmResponse, Message, MessageContent, StreamEvent, ToolCall, ToolDefinition},
    LlmProvider,
};
use tool::Tool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub mime_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryMessage {
    pub role: String,
    pub text: String,
    pub attachments: Option<Vec<Attachment>>,
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
    /// Full reasoning text that preceded a round of tool calls (used for history replay).
    Thinking { text: String },
    /// Real-time streaming chunk of extended-thinking content (arrives before tool calls).
    ThinkingDelta { text: String },
    /// Real-time streaming chunk of the final answer text.
    TextDelta { text: String },
    ToolCall { name: String, input: serde_json::Value },
    ToolResult { name: String, preview: String },
    Done { answer: String, sources: Vec<Source>, tool_calls_made: usize },
    Error { message: String },
    Question { question: String, choices: Vec<String> },
}

pub struct Agent {
    llm: Arc<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    max_iterations: usize,
    compaction_threshold_chars: usize,
    compaction_keep_last: usize,
}

impl Agent {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        tools: Vec<Box<dyn Tool>>,
        max_iterations: usize,
    ) -> Self {
        Self {
            llm,
            tools,
            max_iterations,
            compaction_threshold_chars: usize::MAX,
            compaction_keep_last: 6,
        }
    }

    pub fn with_compaction(mut self, threshold_chars: usize, keep_last: usize) -> Self {
        self.compaction_threshold_chars = threshold_chars;
        self.compaction_keep_last = keep_last;
        self
    }

    pub async fn compact_history(&self, history: &[HistoryMessage]) -> Vec<HistoryMessage> {
        if history.is_empty() || estimate_history_chars(history) <= self.compaction_threshold_chars {
            return history.to_vec();
        }
        let total_messages = history.len();
        let keep_last = self.compaction_keep_last.min(total_messages);
        let old = &history[..total_messages - keep_last];
        let recent = &history[total_messages - keep_last..];

        let conversation_text = old
            .iter()
            .map(|m| format!("[{}]: {}", m.role, m.text))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "Summarize the following conversation concisely, preserving key facts, decisions, \
             and code discussed. This summary will be used as context for continuing the conversation.\n\n\
             {conversation_text}"
        );

        let summary = match self.llm.chat(&[Message::user(prompt)], &[]).await {
            Ok(LlmResponse::Message { text }) => text,
            _ => {
                tracing::warn!("compaction LLM call failed — using full history");
                return history.to_vec();
            }
        };

        tracing::info!(
            old_messages = old.len(),
            kept_messages = keep_last,
            "compacted conversation history"
        );

        let mut result = Vec::with_capacity(1 + keep_last);
        result.push(HistoryMessage { role: "summary".into(), text: summary, attachments: None });
        result.extend_from_slice(recent);
        result
    }

    pub async fn query(
        &self,
        user_query: &str,
        history: &[HistoryMessage],
        attachments: &[Attachment],
    ) -> Result<QueryResponse> {
        let (event_sender, mut receiver) = mpsc::channel::<AgentEvent>(64);
        self.query_streaming(user_query, history, attachments, event_sender).await;

        let mut response = None;
        let mut error = None;
        while let Some(event) = receiver.recv().await {
            match event {
                AgentEvent::Done { answer, sources, tool_calls_made } => {
                    response = Some(QueryResponse { answer, sources, tool_calls_made });
                }
                AgentEvent::Error { message } => {
                    error = Some(anyhow::anyhow!(message));
                }
                _ => {}
            }
        }

        response.ok_or_else(|| error.unwrap_or_else(|| anyhow::anyhow!("agent produced no response")))
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

    /// Generate a one-sentence description of the intent behind a batch of tool calls.
    /// Used to synthesize a Thinking event when the model produces no preamble text.
    async fn describe_tool_calls_batch(&self, calls: &[ToolCall]) -> String {
        if calls.is_empty() {
            return String::new();
        }
        let calls_text = calls
            .iter()
            .map(|c| format!("- {}: {}", c.name, c.input))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "In one sentence, describe what these tool calls are trying to find out. \
             Be concrete and mention key values. No trailing punctuation.\n{calls_text}"
        );
        match self.llm.chat(&[Message::user(prompt)], &[]).await {
            Ok(LlmResponse::Message { text }) => text.trim().to_string(),
            _ => String::new(),
        }
    }

    pub async fn query_streaming(
        &self,
        user_query: &str,
        history: &[HistoryMessage],
        attachments: &[Attachment],
        event_sender: mpsc::Sender<AgentEvent>,
    ) {
        let mut tool_defs: Vec<ToolDefinition> =
            self.tools.iter().map(|t| t.definition()).collect();
        tool_defs.push(ToolDefinition {
            name: "ask_user".to_string(),
            description: "Present a question with predefined choices to the user whenever you \
                          need information to proceed. Use this instead of asking questions in \
                          plain text — never end a response with inline questions or a list of \
                          things you need to know. Call this tool first, then answer once the \
                          user replies. Only skip this tool if the knowledge graph already \
                          contains the answer."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The clarifying question to present to the user."
                    },
                    "choices": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "2–4 concise answer choices for the user to pick from.",
                        "minItems": 2,
                        "maxItems": 4
                    }
                },
                "required": ["question", "choices"]
            }),
        });

        let tool_map: HashMap<String, &dyn Tool> =
            self.tools.iter().map(|t| (t.definition().name, t.as_ref())).collect();

        let compacted = self.compact_history(history).await;
        let mut messages = vec![Message::system(prompt::system_prompt())];
        messages.extend(history_to_messages(&compacted));
        messages.push(build_user_message(user_query, attachments));

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
                    Ok(LlmResponse::ToolCalls { .. }) | Err(_) => break self.last_assistant_text(&messages),
                }
            }

            // Stream the LLM response, collecting events as they arrive.
            let (stream_tx, mut stream_rx) = mpsc::channel::<StreamEvent>(64);
            let llm            = Arc::clone(&self.llm);
            let msgs_snapshot  = messages.clone();
            let tools_snapshot = tool_defs.clone();
            tokio::spawn(async move {
                if let Err(e) = llm.chat_stream(&msgs_snapshot, &tools_snapshot, stream_tx).await {
                    tracing::warn!(error = %e, "chat_stream failed");
                }
            });

            let mut text_buf     = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut stop_reason  = String::new();

            while let Some(ev) = stream_rx.recv().await {
                match ev {
                    StreamEvent::ThinkingDelta { text } => {
                        let _ = event_sender.send(AgentEvent::ThinkingDelta { text }).await;
                    }
                    StreamEvent::TextDelta { text } => {
                        // Forward immediately so preamble appears live; accumulate for Done.
                        let _ = event_sender.send(AgentEvent::TextDelta { text: text.clone() }).await;
                        text_buf.push_str(&text);
                    }
                    StreamEvent::ToolCallReady(call) => {
                        tool_calls.push(call);
                    }
                    StreamEvent::Done { stop_reason: sr } => {
                        stop_reason = sr;
                    }
                }
            }

            if stop_reason == "end_turn" || tool_calls.is_empty() {
                // Text was already streamed as TextDelta events — just finalize.
                break text_buf;
            }

            // Tool-use round: preamble was already streamed live as TextDelta events.
            // The client converts accumulated TextDelta into a Thinking block when the
            // first ToolCall event arrives.
            iterations += 1;

            if let Some(ask) = tool_calls.iter().find(|c| c.name == "ask_user") {
                let question = ask.input["question"].as_str().unwrap_or("").to_string();
                let choices = ask.input["choices"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let _ = event_sender.send(AgentEvent::Question { question, choices }).await;
                let _ = event_sender.send(AgentEvent::Done {
                    answer: text_buf,
                    sources: vec![],
                    tool_calls_made: iterations,
                }).await;
                return;
            }

            let call_parts: Vec<ContentPart> = tool_calls
                .iter()
                .map(|c| ContentPart::ToolUse {
                    id:                c.id.clone(),
                    name:              c.name.clone(),
                    input:             c.input.clone(),
                    thought_signature: c.thought_signature.clone(),
                })
                .collect();
            messages.push(Message {
                role: crate::llm::types::Role::Assistant,
                content: MessageContent::Parts(call_parts),
            });

            // If the model produced no preamble text for this round, synthesize a thinking
            // block so the UI always shows intent before tool calls.
            if text_buf.is_empty() {
                let thinking = self.describe_tool_calls_batch(&tool_calls).await;
                if !thinking.is_empty() {
                    let _ = event_sender.send(AgentEvent::Thinking { text: thinking }).await;
                }
            }

            for call in &tool_calls {
                let _ = event_sender.send(AgentEvent::ToolCall {
                    name:  call.name.clone(),
                    input: call.input.clone(),
                }).await;
            }

            let results = join_all(
                tool_calls.iter().map(|c| self.execute_tool_call(c, &tool_map))
            ).await;

            for (call, result) in tool_calls.iter().zip(results) {
                let preview = tool_map.get(call.name.as_str())
                    .map(|t| t.preview(&result))
                    .unwrap_or_else(|| result.chars().take(tool::DEFAULT_PREVIEW_CHARS).collect());
                let _ = event_sender.send(AgentEvent::ToolResult {
                    name:    call.name.clone(),
                    preview,
                }).await;
                messages.push(Message {
                    role: crate::llm::types::Role::User,
                    content: MessageContent::Parts(vec![ContentPart::ToolResult {
                        tool_use_id: call.id.clone(),
                        content:     result,
                        is_error:    false,
                    }]),
                });
            }
        };

        let sources = parse_citations(&final_text);
        let _ = event_sender.send(AgentEvent::Done {
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
                    ContentPart::Text { text, .. } => Some(text.clone()),
                    _ => None,
                }),
            })
            .unwrap_or_default()
    }
}

pub(crate) fn build_user_message(text: &str, attachments: &[Attachment]) -> Message {
    if attachments.is_empty() {
        return Message::user(text);
    }
    let mut parts = vec![ContentPart::Text { text: text.to_string(), thought_signature: None }];
    for attachment in attachments {
        if attachment.mime_type.starts_with("image/") {
            parts.push(ContentPart::Image {
                media_type: attachment.mime_type.clone(),
                data: attachment.data.clone(),
            });
        } else {
            parts.push(ContentPart::Document {
                media_type: attachment.mime_type.clone(),
                data: attachment.data.clone(),
            });
        }
    }
    Message { role: crate::llm::types::Role::User, content: MessageContent::Parts(parts) }
}

pub(crate) fn estimate_history_chars(history: &[HistoryMessage]) -> usize {
    history.iter().map(|m| m.text.len()).sum()
}

fn history_to_messages(history: &[HistoryMessage]) -> Vec<Message> {
    history.iter().map(|entry| {
        let attachments = entry.attachments.as_deref().unwrap_or(&[]);
        match entry.role.as_str() {
            "assistant" => Message::assistant_text(&entry.text),
            "summary" => Message::user(format!("[Summary of prior conversation]\n{}", entry.text)),
            _ => build_user_message(&entry.text, attachments),
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
        LlmResponse::ToolCalls {
            calls: vec![ToolCall { id: "tc_1".into(), name: name.into(), input: serde_json::json!({}), thought_signature: None }],
            preamble: String::new(),
        }
    }

    fn two_tool_calls(a: &str, b: &str) -> LlmResponse {
        LlmResponse::ToolCalls {
            calls: vec![
                ToolCall { id: "tc_1".into(), name: a.into(), input: serde_json::json!({}), thought_signature: None },
                ToolCall { id: "tc_2".into(), name: b.into(), input: serde_json::json!({}), thought_signature: None },
            ],
            preamble: String::new(),
        }
    }

    fn text(s: &str) -> LlmResponse {
        LlmResponse::Message { text: s.into() }
    }

    fn agent_with(llm: Arc<dyn LlmProvider>, tools: Vec<Box<dyn Tool>>, max: usize) -> Agent {
        Agent::new(llm, tools, max)
    }

    #[test]
    fn user_message_with_no_attachments_is_text_content() {
        let msg = build_user_message("hello", &[]);
        match msg.content {
            MessageContent::Text(t) => assert_eq!(t, "hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn user_message_with_image_attachment_is_parts() {
        let att = Attachment { name: "photo.png".into(), mime_type: "image/png".into(), data: "abc".into() };
        let msg = build_user_message("check this", &[att]);
        match msg.content {
            MessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(&parts[0], ContentPart::Text { text, .. } if text == "check this"));
                assert!(matches!(&parts[1], ContentPart::Image { media_type, .. } if media_type == "image/png"));
            }
            other => panic!("expected Parts, got {other:?}"),
        }
    }

    #[test]
    fn user_message_with_pdf_attachment_is_parts() {
        let att = Attachment { name: "doc.pdf".into(), mime_type: "application/pdf".into(), data: "pdf".into() };
        let msg = build_user_message("read this", &[att]);
        match msg.content {
            MessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(&parts[1], ContentPart::Document { media_type, .. } if media_type == "application/pdf"));
            }
            other => panic!("expected Parts, got {other:?}"),
        }
    }

    #[test]
    fn history_message_with_image_attachment_becomes_parts() {
        let att = Attachment { name: "img.jpg".into(), mime_type: "image/jpeg".into(), data: "data".into() };
        let entry = HistoryMessage { role: "user".into(), text: "see".into(), attachments: Some(vec![att]) };
        let msgs = history_to_messages(&[entry]);
        assert_eq!(msgs.len(), 1);
        assert!(matches!(msgs[0].content, MessageContent::Parts(_)));
    }

    #[test]
    fn history_message_without_attachments_is_text() {
        let entry = HistoryMessage { role: "user".into(), text: "hello".into(), attachments: None };
        let msgs = history_to_messages(&[entry]);
        assert!(matches!(msgs[0].content, MessageContent::Text(_)));
    }

    #[tokio::test]
    async fn text_on_first_turn_returns_immediately() {
        let llm = MockLlm::new(vec![text("all done")]);
        let agent = agent_with(llm, vec![], 5);
        let resp = agent.query("hi", &[], &[]).await.unwrap();
        assert_eq!(resp.answer, "all done");
        assert_eq!(resp.tool_calls_made, 0);
    }

    #[tokio::test]
    async fn one_tool_call_then_text_counts_one_iteration() {
        let llm = MockLlm::new(vec![
            tool_call("my_tool"),
            text("Calling my_tool"),   // consumed by describe_tool_calls_batch
            text("result arrived"),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5);
        let resp = agent.query("hi", &[], &[]).await.unwrap();
        assert_eq!(resp.answer, "result arrived");
        assert_eq!(resp.tool_calls_made, 1);
    }

    #[tokio::test]
    async fn two_tool_call_turns_count_two_iterations() {
        let llm = MockLlm::new(vec![
            tool_call("my_tool"),
            text("Calling my_tool"),   // consumed by describe_tool_calls_batch (round 1)
            tool_call("my_tool"),
            text("Calling my_tool"),   // consumed by describe_tool_calls_batch (round 2)
            text("done after two rounds"),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5);
        let resp = agent.query("hi", &[], &[]).await.unwrap();
        assert_eq!(resp.tool_calls_made, 2);
    }

    #[tokio::test]
    async fn max_iterations_returns_last_assistant_text() {
        let agent = agent_with(
            MockLlm::new(vec![text("partial answer so far")]),
            vec![],
            0,
        );
        let resp = agent.query("hi", &[], &[]).await.unwrap();
        assert_eq!(resp.tool_calls_made, 0);
    }

    #[tokio::test]
    async fn multiple_tool_calls_in_one_turn_all_executed() {
        let llm = MockLlm::new(vec![
            two_tool_calls("tool_a", "tool_b"),
            text("Calling tool_a and tool_b"),  // consumed by describe_tool_calls_batch
            text("got both results"),
        ]);
        let agent = agent_with(
            llm,
            vec![MockTool::new("tool_a", "result_a"), MockTool::new("tool_b", "result_b")],
            5,
        );
        let resp = agent.query("hi", &[], &[]).await.unwrap();
        assert_eq!(resp.answer, "got both results");
        assert_eq!(resp.tool_calls_made, 1);
    }

    #[tokio::test]
    async fn unknown_tool_name_produces_error_string_not_panic() {
        let llm = MockLlm::new(vec![
            tool_call("nonexistent_tool"),
            text("Calling nonexistent_tool"),  // consumed by describe_tool_calls_batch
            text("handled gracefully"),
        ]);
        let agent = agent_with(llm, vec![], 5);
        let resp = agent.query("hi", &[], &[]).await.unwrap();
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

    #[test]
    fn estimate_empty_history_is_zero() {
        assert_eq!(estimate_history_chars(&[]), 0);
    }

    #[test]
    fn estimate_counts_text_chars_of_single_message() {
        let entry = HistoryMessage { role: "user".into(), text: "hello".into(), attachments: None };
        assert_eq!(estimate_history_chars(&[entry]), 5);
    }

    #[test]
    fn estimate_sums_chars_across_messages() {
        let msgs = vec![
            HistoryMessage { role: "user".into(), text: "hi".into(), attachments: None },
            HistoryMessage { role: "assistant".into(), text: "hello".into(), attachments: None },
        ];
        assert_eq!(estimate_history_chars(&msgs), 7);
    }

    #[test]
    fn summary_role_renders_as_user_message_with_prefix() {
        let entry = HistoryMessage { role: "summary".into(), text: "old stuff".into(), attachments: None };
        let msgs = history_to_messages(&[entry]);
        assert_eq!(msgs.len(), 1);
        assert!(matches!(msgs[0].role, crate::llm::types::Role::User));
        match &msgs[0].content {
            MessageContent::Text(t) => {
                assert!(t.contains("old stuff"));
                assert!(t.contains("Summary"));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn compact_returns_unchanged_when_under_threshold() {
        let agent = Agent::new(MockLlm::new(vec![]), vec![], 5)
            .with_compaction(1000, 6);
        let history = vec![
            HistoryMessage { role: "user".into(), text: "short".into(), attachments: None },
        ];
        let result = agent.compact_history(&history).await;
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
    }

    #[tokio::test]
    async fn compact_returns_unchanged_for_empty_history() {
        let agent = Agent::new(MockLlm::new(vec![]), vec![], 5)
            .with_compaction(0, 6);
        let result = agent.compact_history(&[]).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn compact_calls_llm_and_prepends_summary_message() {
        let agent = Agent::new(MockLlm::new(vec![text("summary of old stuff")]), vec![], 5)
            .with_compaction(5, 1);
        let history = vec![
            HistoryMessage { role: "user".into(), text: "message one".into(), attachments: None },
            HistoryMessage { role: "assistant".into(), text: "response one".into(), attachments: None },
            HistoryMessage { role: "user".into(), text: "recent message".into(), attachments: None },
        ];
        let result = agent.compact_history(&history).await;
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "summary");
        assert_eq!(result[0].text, "summary of old stuff");
        assert_eq!(result[1].role, "user");
        assert_eq!(result[1].text, "recent message");
    }

    #[tokio::test]
    async fn compact_keeps_exactly_keep_last_recent_messages() {
        let agent = Agent::new(MockLlm::new(vec![text("summary")]), vec![], 5)
            .with_compaction(5, 2);
        let history: Vec<HistoryMessage> = (0..5).map(|i| HistoryMessage {
            role: "user".into(),
            text: format!("msg {i}"),
            attachments: None,
        }).collect();
        let result = agent.compact_history(&history).await;
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].role, "summary");
        assert_eq!(result[1].text, "msg 3");
        assert_eq!(result[2].text, "msg 4");
    }

    #[tokio::test]
    async fn query_compacts_history_over_threshold() {
        let agent = Agent::new(
            MockLlm::new(vec![text("compact summary"), text("final answer")]),
            vec![],
            5,
        ).with_compaction(5, 1);
        let history = vec![
            HistoryMessage { role: "user".into(), text: "message one".into(), attachments: None },
            HistoryMessage { role: "assistant".into(), text: "response one".into(), attachments: None },
        ];
        let resp = agent.query("new question", &history, &[]).await.unwrap();
        assert_eq!(resp.answer, "final answer");
    }

    #[tokio::test]
    async fn query_streaming_compacts_history_over_threshold() {
        let agent = Arc::new(
            Agent::new(
                MockLlm::new(vec![text("compact summary"), text("streaming answer")]),
                vec![],
                5,
            ).with_compaction(5, 1)
        );
        let history = vec![
            HistoryMessage { role: "user".into(), text: "message one".into(), attachments: None },
            HistoryMessage { role: "assistant".into(), text: "response one".into(), attachments: None },
        ];
        let (event_sender, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        agent.query_streaming("new question", &history, &[], event_sender).await;
        let mut answer = None;
        while let Ok(event) = rx.try_recv() {
            if let AgentEvent::Done { answer: a, .. } = event {
                answer = Some(a);
            }
        }
        assert_eq!(answer.as_deref(), Some("streaming answer"));
    }

    // ── Streaming event tests ──────────────────────────────────────────────────

    async fn collect_agent_events(agent: Arc<Agent>, query: &str) -> Vec<AgentEvent> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(128);
        agent.query_streaming(query, &[], &[], tx).await;
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() { events.push(e); }
        events
    }

    #[tokio::test]
    async fn text_response_emits_text_delta_then_done() {
        let agent = Arc::new(agent_with(MockLlm::new(vec![text("hello world")]), vec![], 5));
        let events = collect_agent_events(agent, "hi").await;

        let deltas: String = events.iter().filter_map(|e| match e {
            AgentEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        }).collect();
        assert_eq!(deltas, "hello world");

        let done = events.iter().any(|e| matches!(e, AgentEvent::Done { .. }));
        assert!(done, "expected Done event");
    }

    #[tokio::test]
    async fn tool_call_preamble_streams_live_as_text_delta() {
        let llm = MockLlm::new(vec![
            LlmResponse::ToolCalls {
                calls: vec![ToolCall { id: "t".into(), name: "my_tool".into(), input: serde_json::json!({}), thought_signature: None }],
                preamble: "Let me check that".into(),
            },
            text("done"),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("my_tool", "result")], 5));
        let events = collect_agent_events(agent, "hi").await;

        // Preamble must stream live as TextDelta, not be batched into a Thinking event.
        let preamble_pos = events.iter().position(|e| matches!(e, AgentEvent::TextDelta { text } if text == "Let me check that"))
            .expect("expected TextDelta with preamble text");
        let tool_pos = events.iter().position(|e| matches!(e, AgentEvent::ToolCall { name, .. } if name == "my_tool"))
            .expect("expected ToolCall event");
        assert!(preamble_pos < tool_pos, "preamble TextDelta must arrive before ToolCall");

        let has_thinking = events.iter().any(|e| matches!(e, AgentEvent::Thinking { .. }));
        assert!(!has_thinking, "no consolidated Thinking event — preamble streams live as TextDelta");
    }

    #[tokio::test]
    async fn no_preamble_synthesizes_thinking_event_before_tool_call() {
        // The LLM returns tool calls with no preamble text; then the describe_tool_calls_batch
        // call returns a description; then the final text answer arrives.
        let llm = MockLlm::new(vec![
            tool_call("my_tool"),
            text("batch description"),  // used by describe_tool_calls_batch
            text("done"),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5));
        let events = collect_agent_events(agent, "hi").await;

        // A Thinking event must appear before the ToolCall event.
        let thinking_pos = events.iter().position(|e| matches!(e, AgentEvent::Thinking { .. }))
            .expect("expected a synthesized Thinking event when no preamble");
        let tool_pos = events.iter().position(|e| matches!(e, AgentEvent::ToolCall { .. }))
            .expect("expected ToolCall event");
        assert!(thinking_pos < tool_pos, "Thinking must precede ToolCall");

        // No TextDelta should appear before the ToolCall.
        let has_delta_before_tool = events[..tool_pos].iter().any(|e| matches!(e, AgentEvent::TextDelta { .. }));
        assert!(!has_delta_before_tool, "no TextDelta expected before tool call when preamble is empty");
    }

    #[tokio::test]
    async fn text_delta_events_reassemble_to_full_answer() {
        let agent = Arc::new(agent_with(MockLlm::new(vec![text("The answer is 42")]), vec![], 5));
        let events = collect_agent_events(agent, "hi").await;

        let reassembled: String = events.iter().filter_map(|e| match e {
            AgentEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        }).collect();
        assert_eq!(reassembled, "The answer is 42");

        let done_answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.as_str()),
            _ => None,
        });
        assert_eq!(done_answer, Some("The answer is 42"));
    }
}

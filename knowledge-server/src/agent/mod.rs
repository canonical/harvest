pub mod artifact_tools;
pub mod chain;
pub mod deployment_tools;
pub mod graph_tools;
pub mod issue_tools;
pub mod lxd_tools;
pub mod machine_tools;
pub mod port_forward_tools;
pub mod skill_tools;
pub mod terraform_tools;
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
    types::{
        ContentPart, LlmResponse, Message, MessageContent, ProviderSelection, StreamEvent,
        ToolCall, ToolDefinition, UsedProvider,
    },
    LlmProvider,
};
use tool::Tool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub mime_type: String,
    #[serde(default)]
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
    pub provider_used: Option<UsedProvider>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntentMode {
    Conversational,
    Research,
    Action,
    Hybrid,
}

impl Default for IntentMode {
    fn default() -> Self { Self::Research }
}

impl IntentMode {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "conversational" | "answer" => Self::Conversational,
            "action" | "execute" => Self::Action,
            "hybrid" => Self::Hybrid,
            _ => Self::Research,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    Intent { mode: IntentMode },
    Phase { label: String },
    Thinking { text: String },
    ThinkingDelta { text: String },
    TextDelta { text: String },
    ToolCall { name: String, input: serde_json::Value },
    ToolResult { name: String, preview: String },
    Done {
        answer: String,
        sources: Vec<Source>,
        tool_calls_made: usize,
        provider_used: Option<UsedProvider>,
    },
    Error { message: String },
    Question { question: String, choices: Vec<String> },
    ConfirmAction { id: String, name: String, input: serde_json::Value, description: String },
    TitleUpdated { title: String },
}

enum LoopOutcome {
    Finished { text: String, iterations: usize, provider_used: Option<UsedProvider> },
    EndedWithoutCitations { text: String, iterations: usize, provider_used: Option<UsedProvider> },
    Paused {
        messages: Vec<Message>,
        iterations: usize,
        text_buf: String,
        pending: Vec<PendingConfirmCall>,
        provider_used: Option<UsedProvider>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingConfirmCall {
    pub id:          String,
    pub tool_use_id: String,
}

pub struct PausedTurn {
    pub messages:   Vec<Message>,
    pub iterations: usize,
    pub pending:    Vec<PendingConfirmCall>,
}

pub struct ToolResumeResult {
    pub tool_call_id: String,
    pub content:       String,
    pub is_error:      bool,
}

pub struct Agent {
    llm: Arc<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    max_iterations: usize,
    compaction_threshold_chars: usize,
    compaction_keep_last: usize,
    system_prompt_override: Option<String>,
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
            system_prompt_override: None,
        }
    }

    pub fn llm(&self) -> &Arc<dyn LlmProvider> {
        &self.llm
    }

    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt_override = Some(prompt);
        self
    }

    fn effective_system_prompt(&self) -> String {
        self.system_prompt_override.clone().unwrap_or_else(prompt::system_prompt)
    }

    pub fn with_compaction(mut self, threshold_chars: usize, keep_last: usize) -> Self {
        self.compaction_threshold_chars = threshold_chars;
        self.compaction_keep_last = keep_last;
        self
    }

    pub async fn classify_intent(
        &self,
        user_query: &str,
        history: &[HistoryMessage],
        selection: Option<&ProviderSelection>,
    ) -> IntentMode {
        if self.tools.is_empty() {
            return IntentMode::Conversational;
        }
        if history.is_empty() && !user_query.trim().is_empty() {
            let lower = user_query.to_lowercase();
            let action_verbs = [
                "run ", "restart", "deploy", "install", "execute",
                "stop ", "start ", "create ", "delete ", "update ",
                "provision", "destroy", "apply ",
            ];
            if action_verbs.iter().any(|v| lower.contains(v)) {
                if lower.contains("find") || lower.contains("search") || lower.contains("how") {
                    return IntentMode::Hybrid;
                }
                return IntentMode::Action;
            }
        }
        let snippet = history.iter()
            .rev()
            .take(3)
            .rev()
            .map(|m| format!("[{}]: {}", m.role, m.text.chars().take(200).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = prompt::intent_classifier_prompt(user_query, &snippet);
        let messages = vec![Message::user(prompt)];
        match self.llm.chat_routed(selection, &messages, &[]).await {
            Ok((LlmResponse::Message { text }, _)) => IntentMode::from_str(&text),
            _ => IntentMode::default(),
        }
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
        selection: Option<&ProviderSelection>,
    ) -> Result<QueryResponse> {
        let (event_sender, mut receiver) = mpsc::channel::<AgentEvent>(64);
        self.query_streaming(user_query, history, attachments, selection, event_sender).await;

        let mut response = None;
        let mut error = None;
        while let Some(event) = receiver.recv().await {
            match event {
                AgentEvent::Done { answer, sources, tool_calls_made, provider_used } => {
                    response = Some(QueryResponse { answer, sources, tool_calls_made, provider_used });
                }
                AgentEvent::Error { message } => {
                    error = Some(anyhow::anyhow!(message));
                }
                _ => {}
            }
        }

        response.ok_or_else(|| error.unwrap_or_else(|| anyhow::anyhow!("agent produced no response")))
    }

    pub async fn query_with_progress(
        &self,
        user_query: &str,
        history: &[HistoryMessage],
        attachments: &[Attachment],
        selection: Option<&ProviderSelection>,
        progress: mpsc::Sender<AgentEvent>,
    ) -> Result<QueryResponse> {
        let (event_sender, mut receiver) = mpsc::channel::<AgentEvent>(64);
        self.query_streaming(user_query, history, attachments, selection, event_sender).await;

        let mut response = None;
        let mut error = None;
        while let Some(event) = receiver.recv().await {
            let _ = progress.send(event.clone()).await;
            match event {
                AgentEvent::Done { answer, sources, tool_calls_made, provider_used } => {
                    response = Some(QueryResponse { answer, sources, tool_calls_made, provider_used });
                }
                AgentEvent::Error { message } => {
                    error = Some(anyhow::anyhow!(message));
                }
                _ => {}
            }
        }

        response.ok_or_else(|| error.unwrap_or_else(|| anyhow::anyhow!("agent produced no response")))
    }

    fn build_tool_defs(&self) -> Vec<ToolDefinition> {
        let mut tool_defs: Vec<ToolDefinition> =
            self.tools.iter().map(|t| t.definition()).collect();
        tool_defs.push(ask_user_tool_def());
        tool_defs
    }

    fn build_tool_map(&self) -> HashMap<String, &dyn Tool> {
        self.tools.iter().map(|t| (t.definition().name, t.as_ref())).collect()
    }

    pub async fn query_streaming(
        &self,
        user_query: &str,
        history: &[HistoryMessage],
        attachments: &[Attachment],
        selection: Option<&ProviderSelection>,
        event_sender: mpsc::Sender<AgentEvent>,
    ) -> Option<PausedTurn> {
        let mode = self.classify_intent(user_query, history, selection).await;
        let _ = event_sender.send(AgentEvent::Intent { mode }).await;

        let (tool_defs, tool_map) = match mode {
            IntentMode::Conversational => (vec![ask_user_tool_def()], HashMap::new()),
            _ => (self.build_tool_defs(), self.build_tool_map()),
        };

        let compacted = self.compact_history(history).await;
        let mut messages = vec![Message::system(self.effective_system_prompt())];
        messages.extend(history_to_messages(&compacted));
        messages.push(build_user_message(user_query, attachments));

        let outcome = self.run_loop(messages, 0, &tool_defs, &tool_map, selection, &event_sender).await;
        self.finish_outcome(outcome, &event_sender).await
    }

    pub async fn resume_after_confirm(
        &self,
        mut messages: Vec<Message>,
        iterations: usize,
        results: Vec<ToolResumeResult>,
        selection: Option<&ProviderSelection>,
        event_sender: mpsc::Sender<AgentEvent>,
    ) -> Option<PausedTurn> {
        for r in results {
            messages.push(Message {
                role: crate::llm::types::Role::User,
                content: MessageContent::Parts(vec![ContentPart::ToolResult {
                    tool_use_id: r.tool_call_id,
                    content:     r.content,
                    is_error:    r.is_error,
                }]),
            });
        }

        let tool_defs = self.build_tool_defs();
        let tool_map  = self.build_tool_map();

        let outcome = self.run_loop(messages, iterations, &tool_defs, &tool_map, selection, &event_sender).await;
        self.finish_outcome(outcome, &event_sender).await
    }

    async fn finish_outcome(
        &self,
        outcome: LoopOutcome,
        event_sender: &mpsc::Sender<AgentEvent>,
    ) -> Option<PausedTurn> {
        match outcome {
            LoopOutcome::Finished { text, iterations, provider_used } => {
                let answer = if text.is_empty() { last_resort_fallback() } else { text };
                let sources = parse_citations(&answer);
                let _ = event_sender.send(AgentEvent::Done {
                    answer,
                    sources,
                    tool_calls_made: iterations,
                    provider_used,
                }).await;
                None
            }
            LoopOutcome::EndedWithoutCitations { text, iterations, provider_used } => {
                let answer = if text.is_empty() { question_fallback() } else { text };
                let _ = event_sender.send(AgentEvent::Done {
                    answer,
                    sources: vec![],
                    tool_calls_made: iterations,
                    provider_used,
                }).await;
                None
            }
            LoopOutcome::Paused { messages, iterations, text_buf, pending, provider_used } => {
                let answer = if text_buf.is_empty() { question_fallback() } else { text_buf };
                let _ = event_sender.send(AgentEvent::Done {
                    answer,
                    sources: vec![],
                    tool_calls_made: iterations,
                    provider_used,
                }).await;
                Some(PausedTurn { messages, iterations, pending })
            }
        }
    }

    async fn run_loop(
        &self,
        mut messages: Vec<Message>,
        mut iterations: usize,
        tool_defs: &[ToolDefinition],
        tool_map: &HashMap<String, &dyn Tool>,
        selection: Option<&ProviderSelection>,
        event_sender: &mpsc::Sender<AgentEvent>,
    ) -> LoopOutcome {
        let mut last_provider_used: Option<UsedProvider> = None;
        let mut accumulated_text = String::new();
        loop {
            if iterations >= self.max_iterations {
                tracing::warn!("agent hit max_iterations={} — requesting synthesis", self.max_iterations);
                let _ = event_sender.send(AgentEvent::Phase { label: "Synthesizing answer".to_string() }).await;
                let tool_summary = collect_tool_result_summary(&messages);
                let synthesis_prompt = format!(
                    "You have used the maximum number of tool calls. \
                     Synthesize what you have gathered so far into a final answer.\n\n\
                     Tool results so far:\n{tool_summary}"
                );
                messages.push(Message::user(synthesis_prompt));
                let text = match self.llm.chat_routed(selection, &messages, &[]).await {
                    Ok((LlmResponse::Message { text }, used)) => {
                        last_provider_used = Some(used);
                        text
                    }
                    Ok((LlmResponse::ToolCalls { preamble, .. }, used)) => {
                        last_provider_used = Some(used);
                        if !preamble.is_empty() { preamble }
                        else if !accumulated_text.is_empty() { accumulated_text }
                        else { last_resort_fallback() }
                    }
                    Err(_) => {
                        if !accumulated_text.is_empty() { accumulated_text } else { last_resort_fallback() }
                    }
                };
                return LoopOutcome::Finished { text, iterations, provider_used: last_provider_used };
            }

            let (stream_tx, mut stream_rx) = mpsc::channel::<StreamEvent>(64);
            let llm            = Arc::clone(&self.llm);
            let msgs_snapshot  = messages.clone();
            let tools_snapshot = tool_defs.to_vec();
            let selection_owned = selection.cloned();
            let stream_handle = tokio::spawn(async move {
                llm.chat_stream_routed(selection_owned.as_ref(), &msgs_snapshot, &tools_snapshot, stream_tx).await
            });

            let mut text_buf     = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut stop_reason  = String::new();
            let mut thinking_streamed = false;

            while let Some(ev) = stream_rx.recv().await {
                match ev {
                    StreamEvent::ThinkingDelta { text } => {
                        thinking_streamed = true;
                        let _ = event_sender.send(AgentEvent::ThinkingDelta { text }).await;
                    }
                    StreamEvent::TextDelta { text } => {
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

            match stream_handle.await {
                Ok(Ok(used)) => last_provider_used = Some(used),
                Ok(Err(e)) => tracing::warn!(error = %e, "chat_stream failed"),
                Err(e) => tracing::warn!(error = %e, "chat_stream task panicked"),
            }

            if !text_buf.is_empty() {
                accumulated_text.push_str(&text_buf);
            }

            if stop_reason == "end_turn" || tool_calls.is_empty() {
                // Fix 2: Check whether the LLM emitted an ask_user call as a JSON
                // code block in its text instead of using the proper tool-calling
                // mechanism. This happens with some models (e.g. Gemini Flash) when
                // they know about the tool from the system prompt but did not receive
                // it as a declared function — or simply got confused.
                if let Some((question, choices, cleaned)) = extract_text_ask_user(&text_buf) {
                    let answer_text = if !cleaned.is_empty() {
                        cleaned
                    } else if !accumulated_text.is_empty() {
                        accumulated_text.clone()
                    } else {
                        // Fix 4: Synthesize a partial answer from gathered context.
                        match self.synthesize_partial_answer(&messages, selection).await {
                            Some(synth) => synth,
                            None => question.clone(),
                        }
                    };
                    let _ = event_sender.send(AgentEvent::Question { question, choices }).await;
                    return LoopOutcome::EndedWithoutCitations {
                        text: answer_text, iterations, provider_used: last_provider_used,
                    };
                }
                return LoopOutcome::Finished { text: text_buf, iterations, provider_used: last_provider_used };
            }

            iterations += 1;

            if let Some(ask) = tool_calls.iter().find(|c| c.name == "ask_user") {
                let question = ask.input["question"].as_str().unwrap_or("").to_string();
                let choices = ask.input["choices"]
                    .as_array()
                    .map(|a| a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .filter(|s| !is_catchall(s))
                        .collect())
                    .unwrap_or_default();
                // Fix 3: Use accumulated_text when text_buf is empty so the user
                // sees the findings from prior iterations alongside the question.
                let answer_text = if !text_buf.is_empty() {
                    text_buf
                } else if !accumulated_text.is_empty() {
                    accumulated_text.clone()
                } else {
                    // Fix 4: Synthesize a partial answer from gathered context.
                    match self.synthesize_partial_answer(&messages, selection).await {
                        Some(synth) => synth,
                        None => question.clone(),
                    }
                };
                let _ = event_sender.send(AgentEvent::Question { question, choices }).await;
                return LoopOutcome::EndedWithoutCitations {
                    text: answer_text, iterations, provider_used: last_provider_used,
                };
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

            let (confirmable, automatic): (Vec<&ToolCall>, Vec<&ToolCall>) = tool_calls.iter().partition(|c| {
                tool_map.get(c.name.as_str()).map(|t| t.requires_confirmation()).unwrap_or(false)
            });

            if !confirmable.is_empty() {
                let mut pending = Vec::with_capacity(confirmable.len());
                let confirm_description = if !text_buf.is_empty() {
                    Some(text_buf.clone())
                } else {
                    None
                };
                for (idx, call) in confirmable.iter().enumerate() {
                    let ui_id = format!("{}:{idx}", call.id);
                    let _ = event_sender.send(AgentEvent::ConfirmAction {
                        id:          ui_id.clone(),
                        name:        call.name.clone(),
                        input:       call.input.clone(),
                        description: confirm_description.clone().unwrap_or_default(),
                    }).await;
                    pending.push(PendingConfirmCall { id: ui_id, tool_use_id: call.id.clone() });
                }

                if !automatic.is_empty() {
                    for call in &automatic {
                        let _ = event_sender.send(AgentEvent::ToolCall {
                            name:  call.name.clone(),
                            input: call.input.clone(),
                        }).await;
                    }
                    let results = join_all(
                        automatic.iter().map(|c| self.execute_tool_call(c, tool_map))
                    ).await;
                    for (call, result) in automatic.iter().zip(results) {
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
                }

                let paused_text = if !text_buf.is_empty() {
                    text_buf
                } else {
                    accumulated_text.clone()
                };
                return LoopOutcome::Paused { messages, iterations, text_buf: paused_text, pending, provider_used: last_provider_used };
            }

            let phase = derive_phase(&tool_calls);
            let _ = event_sender.send(AgentEvent::Phase { label: phase.to_string() }).await;

            for call in &tool_calls {
                let _ = event_sender.send(AgentEvent::ToolCall {
                    name:  call.name.clone(),
                    input: call.input.clone(),
                }).await;
            }

            let results = join_all(
                tool_calls.iter().map(|c| self.execute_tool_call(c, tool_map))
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
        }
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

    /// Synthesize a short partial answer from the tool results gathered so far.
    /// Used when the LLM calls `ask_user` with no preamble text and no text was
    /// accumulated in prior iterations — ensures the user gets a useful answer
    /// body alongside the follow-up question.
    async fn synthesize_partial_answer(
        &self,
        messages: &[Message],
        selection: Option<&ProviderSelection>,
    ) -> Option<String> {
        let tool_summary = collect_tool_result_summary(messages);
        if tool_summary == "No tool results were collected." {
            return None;
        }
        let prompt = format!(
            "You have gathered information from the codebase and are about to ask the user \
             a follow-up question. In 1–3 sentences, briefly summarize what you've found so far \
             so the user has context for the question. Do not ask any questions yourself.\n\n\
             Tool results so far:\n{tool_summary}"
        );
        let mut synth_messages = messages.to_vec();
        synth_messages.push(Message::user(prompt));
        match self.llm.chat_routed(selection, &synth_messages, &[]).await {
            Ok((LlmResponse::Message { text }, _)) if !text.is_empty() => Some(text),
            Ok((LlmResponse::ToolCalls { preamble, .. }, _)) if !preamble.is_empty() => Some(preamble),
            _ => None,
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

fn collect_tool_result_summary(messages: &[Message]) -> String {
    let mut summaries = Vec::new();
    for msg in messages {
        if let MessageContent::Parts(parts) = &msg.content {
            for part in parts {
                if let ContentPart::ToolResult { content, is_error, .. } = part {
                    let label = if *is_error { "error" } else { "result" };
                    let snippet = if content.len() > 200 {
                        format!("{}…", &content[..200])
                    } else {
                        content.clone()
                    };
                    summaries.push(format!("- [{label}] {snippet}"));
                }
            }
        }
    }
    if summaries.is_empty() {
        "No tool results were collected.".to_string()
    } else {
        summaries.join("\n")
    }
}

fn last_resort_fallback() -> String {
    "I reached the tool-call limit before completing my analysis. \
     Please ask a more specific question or try again."
        .to_string()
}

fn question_fallback() -> String {
    "I've gathered what I can from the codebase. \
     Please answer the question above so I can give you a precise answer."
        .to_string()
}

fn derive_phase(tool_calls: &[ToolCall]) -> &'static str {
    let has = |name: &str| tool_calls.iter().any(|c| c.name == name);

    if has("find_callers") || has("find_callees") || has("run_cypher") {
        "Tracing relationships"
    } else if has("get_symbol_source") || has("get_file_symbols") || has("get_imports") || has("compare_symbol_across_versions") {
        "Reading source"
    } else if has("list_repositories") || has("search_symbols") {
        "Searching codebase"
    } else if has("run_command") || has("list_agents") {
        "Executing on agents"
    } else {
        "Working"
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

fn ask_user_tool_def() -> ToolDefinition {
    ToolDefinition {
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
    }
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
    let re = Regex::new(r"\[([^:\]\s]+):([^:\]\s]+):([^:\]\s]+):(\d+)(?:-\d+)?\]").unwrap();
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
        id: String,
        responses: Mutex<VecDeque<LlmResponse>>,
    }

    impl MockLlm {
        fn new(responses: Vec<LlmResponse>) -> Arc<Self> {
            Self::with_id("mock-llm", responses)
        }

        fn with_id(id: &str, responses: Vec<LlmResponse>) -> Arc<Self> {
            Arc::new(Self { id: id.into(), responses: Mutex::new(responses.into()) })
        }
    }

    #[async_trait]
    impl LlmProvider for MockLlm {
        fn id(&self) -> &str { &self.id }
        fn kind(&self) -> &str { "mock" }
        fn default_model(&self) -> &str { "mock-model" }

        async fn list_models(&self) -> Result<Vec<crate::llm::types::ModelInfo>> { Ok(vec![]) }

        async fn chat_with(&self, _model: Option<&str>, _messages: &[Message], _tools: &[ToolDefinition]) -> Result<LlmResponse> {
            self.responses.lock().unwrap().pop_front()
                .ok_or_else(|| anyhow::anyhow!("MockLlm: no more responses"))
        }
    }

    /// Mimics a provider (like Gemini) that streams real `ThinkingDelta` events
    /// directly, rather than only returning a final response for `chat_with`.
    struct MockStreamingLlm {
        rounds: Mutex<VecDeque<Vec<StreamEvent>>>,
    }

    impl MockStreamingLlm {
        fn new(rounds: Vec<Vec<StreamEvent>>) -> Arc<Self> {
            Arc::new(Self { rounds: Mutex::new(rounds.into()) })
        }
    }

    #[async_trait]
    impl LlmProvider for MockStreamingLlm {
        fn id(&self) -> &str { "mock-streaming-llm" }
        fn kind(&self) -> &str { "mock" }
        fn default_model(&self) -> &str { "mock-model" }

        async fn list_models(&self) -> Result<Vec<crate::llm::types::ModelInfo>> { Ok(vec![]) }

        async fn chat_with(&self, _model: Option<&str>, _messages: &[Message], _tools: &[ToolDefinition]) -> Result<LlmResponse> {
            Err(anyhow::anyhow!("MockStreamingLlm only supports chat_stream_with"))
        }

        async fn chat_stream_with(
            &self,
            _model: Option<&str>,
            _messages: &[Message],
            _tools: &[ToolDefinition],
            tx: mpsc::Sender<StreamEvent>,
        ) -> Result<()> {
            let events = self.rounds.lock().unwrap().pop_front()
                .ok_or_else(|| anyhow::anyhow!("MockStreamingLlm: no more rounds"))?;
            for event in events {
                let _ = tx.send(event).await;
            }
            Ok(())
        }
    }

    struct MockTool {
        name: String,
        returns: String,
        confirm: bool,
    }

    impl MockTool {
        fn new(name: &str, returns: &str) -> Box<Self> {
            Box::new(Self { name: name.into(), returns: returns.into(), confirm: false })
        }

        fn new_confirmable(name: &str, returns: &str) -> Box<Self> {
            Box::new(Self { name: name.into(), returns: returns.into(), confirm: true })
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
        fn requires_confirmation(&self) -> bool {
            self.confirm
        }
    }

    fn tool_call(name: &str) -> LlmResponse {
        LlmResponse::ToolCalls {
            calls: vec![ToolCall { id: "tc_1".into(), name: name.into(), input: serde_json::json!({}), thought_signature: None }],
            preamble: String::new(),
        }
    }

    fn tool_call_obj(name: &str, input: impl Into<serde_json::Value>) -> ToolCall {
        ToolCall { id: "tc_1".into(), name: name.into(), input: input.into(), thought_signature: None }
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
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "all done");
        assert_eq!(resp.tool_calls_made, 0);
    }

    #[tokio::test]
    async fn no_selection_reports_default_provider_used() {
        let llm = MockLlm::with_id("mock-llm", vec![text("all done")]);
        let agent = agent_with(llm, vec![], 5);
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        let used = resp.provider_used.expect("expected provider_used to be set");
        assert_eq!(used.provider_id, "mock-llm");
        assert_eq!(used.model, "mock-model");
    }

    #[tokio::test]
    async fn matching_selection_reports_overridden_model() {
        let llm = MockLlm::with_id("mock-llm", vec![text("all done")]);
        let agent = agent_with(llm, vec![], 5);
        let selection = ProviderSelection { provider_id: "mock-llm".into(), model: Some("custom-model".into()) };
        let resp = agent.query("hi", &[], &[], Some(&selection)).await.unwrap();
        let used = resp.provider_used.expect("expected provider_used to be set");
        assert_eq!(used.provider_id, "mock-llm");
        assert_eq!(used.model, "custom-model");
    }

    #[tokio::test]
    async fn one_tool_call_then_text_counts_one_iteration() {
        let llm = MockLlm::new(vec![
            text("research"),
            tool_call("my_tool"),
            text("result arrived"),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5);
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "result arrived");
        assert_eq!(resp.tool_calls_made, 1);
    }

    #[tokio::test]
    async fn query_with_progress_returns_the_same_response_as_query() {
        let llm = MockLlm::new(vec![
            text("research"),
            tool_call("my_tool"),
            text("result arrived"),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5);
        let (tx, _rx) = mpsc::channel::<AgentEvent>(64);
        let resp = agent.query_with_progress("hi", &[], &[], None, tx).await.unwrap();
        assert_eq!(resp.answer, "result arrived");
        assert_eq!(resp.tool_calls_made, 1);
    }

    #[tokio::test]
    async fn query_with_progress_forwards_tool_call_and_done_events() {
        let llm = MockLlm::new(vec![
            text("research"),
            tool_call("my_tool"),
            text("result arrived"),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5);
        let (tx, mut rx) = mpsc::channel::<AgentEvent>(64);
        agent.query_with_progress("hi", &[], &[], None, tx).await.unwrap();

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolCall { name, .. } if name == "my_tool")));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Done { answer, .. } if answer == "result arrived")));
    }

    #[tokio::test]
    async fn two_tool_call_turns_count_two_iterations() {
        let llm = MockLlm::new(vec![
            text("research"),
            tool_call("my_tool"),
            tool_call("my_tool"),
            text("done after two rounds"),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5);
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert_eq!(resp.tool_calls_made, 2);
    }

    #[tokio::test]
    async fn unknown_tool_name_produces_error_string_not_panic() {
        let llm = MockLlm::new(vec![
            text("research"),
            tool_call("nonexistent_tool"),
            text("handled gracefully"),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("some_tool", "ok")], 5);
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "handled gracefully");
    }

    #[tokio::test]
    async fn max_iterations_returns_last_assistant_text() {
        let agent = agent_with(
            MockLlm::new(vec![text("partial answer so far")]),
            vec![],
            0,
        );
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert_eq!(resp.tool_calls_made, 0);
    }

    #[tokio::test]
    async fn max_iterations_produces_non_empty_fallback_when_no_text() {
        let llm = MockLlm::new(vec![
            tool_call("my_tool"),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "result")], 1);
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert!(!resp.answer.is_empty(), "answer must not be empty when max_iterations is hit");
    }

    #[tokio::test]
    async fn max_iterations_preserves_accumulated_text() {
        let llm = MockLlm::new(vec![
            LlmResponse::ToolCalls {
                calls: vec![ToolCall { id: "tc_1".into(), name: "my_tool".into(), input: serde_json::json!({}), thought_signature: None }],
                preamble: "I found something".into(),
            },
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "result")], 1);
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert!(!resp.answer.is_empty());
        assert!(resp.answer.contains("I found something") || !resp.answer.is_empty());
    }

    #[tokio::test]
    async fn classify_intent_returns_action_for_run_command() {
        let llm = MockLlm::new(vec![]);
        let agent = agent_with(llm, vec![MockTool::new("run_command", "ok")], 5);
        let mode = agent.classify_intent("restart nginx on build-box", &[], None).await;
        assert_eq!(mode, IntentMode::Action);
    }

    #[tokio::test]
    async fn classify_intent_returns_hybrid_for_find_and_update() {
        let llm = MockLlm::new(vec![]);
        let agent = agent_with(llm, vec![MockTool::new("run_command", "ok")], 5);
        let mode = agent.classify_intent("find the config file and update the timeout", &[], None).await;
        assert_eq!(mode, IntentMode::Hybrid);
    }

    #[tokio::test]
    async fn classify_intent_returns_research_for_how_question() {
        let llm = MockLlm::new(vec![text("research")]);
        let agent = agent_with(llm, vec![MockTool::new("search_symbols", "ok")], 5);
        let mode = agent.classify_intent("how does the retry logic work?", &[], None).await;
        assert_eq!(mode, IntentMode::Research);
    }

    #[test]
    fn derive_phase_searching_for_search_symbols() {
        let calls = vec![tool_call_obj("search_symbols", "retry")];
        assert_eq!(derive_phase(&calls), "Searching codebase");
    }

    #[test]
    fn derive_phase_searching_for_list_repositories() {
        let calls = vec![tool_call_obj("list_repositories", serde_json::json!({}))];
        assert_eq!(derive_phase(&calls), "Searching codebase");
    }

    #[test]
    fn derive_phase_reading_source_for_get_symbol_source() {
        let calls = vec![tool_call_obj("get_symbol_source", "retry_loop")];
        assert_eq!(derive_phase(&calls), "Reading source");
    }

    #[test]
    fn derive_phase_reading_source_for_get_file_symbols() {
        let calls = vec![tool_call_obj("get_file_symbols", "oidc.py")];
        assert_eq!(derive_phase(&calls), "Reading source");
    }

    #[test]
    fn derive_phase_reading_source_for_get_imports() {
        let calls = vec![tool_call_obj("get_imports", "oidc.py")];
        assert_eq!(derive_phase(&calls), "Reading source");
    }

    #[test]
    fn derive_phase_tracing_for_find_callers() {
        let calls = vec![tool_call_obj("find_callers", "retry_loop")];
        assert_eq!(derive_phase(&calls), "Tracing relationships");
    }

    #[test]
    fn derive_phase_tracing_for_find_callees() {
        let calls = vec![tool_call_obj("find_callees", "retry_loop")];
        assert_eq!(derive_phase(&calls), "Tracing relationships");
    }

    #[test]
    fn derive_phase_tracing_for_run_cypher() {
        let calls = vec![tool_call_obj("run_cypher", "MATCH (n) RETURN n")];
        assert_eq!(derive_phase(&calls), "Tracing relationships");
    }

    #[test]
    fn derive_phase_executing_for_run_command() {
        let calls = vec![tool_call_obj("run_command", "ls -la")];
        assert_eq!(derive_phase(&calls), "Executing on agents");
    }

    #[test]
    fn derive_phase_executing_for_list_agents() {
        let calls = vec![tool_call_obj("list_agents", serde_json::json!({}))];
        assert_eq!(derive_phase(&calls), "Executing on agents");
    }

    #[test]
    fn derive_phase_tracing_takes_priority_over_searching() {
        let calls = vec![
            tool_call_obj("search_symbols", "retry"),
            tool_call_obj("find_callers", "retry_loop"),
        ];
        assert_eq!(derive_phase(&calls), "Tracing relationships");
    }

    #[test]
    fn derive_phase_reading_takes_priority_over_searching() {
        let calls = vec![
            tool_call_obj("search_symbols", "retry"),
            tool_call_obj("get_symbol_source", "retry_loop"),
        ];
        assert_eq!(derive_phase(&calls), "Reading source");
    }

    #[test]
    fn derive_phase_unknown_tool_returns_working() {
        let calls = vec![tool_call_obj("some_unknown_tool", serde_json::json!({}))];
        assert_eq!(derive_phase(&calls), "Working");
    }

    #[test]
    fn derive_phase_empty_calls_returns_working() {
        let calls: Vec<ToolCall> = vec![];
        assert_eq!(derive_phase(&calls), "Working");
    }

    #[tokio::test]
    async fn query_streaming_emits_phase_event_before_tool_execution() {
        let llm = MockLlm::new(vec![
            text("research"),
            tool_call("search_symbols"),
            text("final answer"),
        ]);
        let agent = Arc::new(agent_with(
            llm,
            vec![MockTool::new("search_symbols", "ok")],
            5,
        ));
        let events = collect_agent_events(agent, "how does X work?").await;

        let phase_pos = events.iter().position(|e| matches!(e, AgentEvent::Phase { label } if label == "Searching codebase"))
            .expect("expected a Phase event with 'Searching codebase'");
        let tool_pos = events.iter().position(|e| matches!(e, AgentEvent::ToolCall { .. }))
            .expect("expected a ToolCall event");
        assert!(phase_pos < tool_pos, "Phase event must arrive before ToolCall");
    }

    #[tokio::test]
    async fn query_streaming_emits_updated_phase_when_tools_change() {
        let llm = MockLlm::new(vec![
            text("research"),
            tool_call("search_symbols"),
            tool_call("get_symbol_source"),
            text("final answer"),
        ]);
        let agent = Arc::new(agent_with(
            llm,
            vec![MockTool::new("search_symbols", "ok"), MockTool::new("get_symbol_source", "source")],
            5,
        ));
        let events = collect_agent_events(agent, "how does X work?").await;

        let phases: Vec<String> = events.iter().filter_map(|e| match e {
            AgentEvent::Phase { label } => Some(label.clone()),
            _ => None,
        }).collect();

        assert!(phases.contains(&"Searching codebase".to_string()), "should emit 'Searching codebase' phase");
        assert!(phases.contains(&"Reading source".to_string()), "should emit 'Reading source' phase");
    }

    #[tokio::test]
    async fn query_streaming_emits_synthesizing_phase_on_max_iterations() {
        let llm = MockLlm::new(vec![
            text("research"),
            tool_call("search_symbols"),
        ]);
        let agent = Arc::new(agent_with(
            llm,
            vec![MockTool::new("search_symbols", "ok")],
            1,
        ));
        let events = collect_agent_events(agent, "how does X work?").await;

        let has_synthesizing = events.iter().any(|e| matches!(e, AgentEvent::Phase { label } if label == "Synthesizing answer"));
        assert!(has_synthesizing, "should emit 'Synthesizing answer' phase when max_iterations is hit");
    }

    #[tokio::test]
    async fn classify_intent_returns_conversational_from_llm() {
        let llm = MockLlm::new(vec![text("conversational")]);
        let agent = agent_with(llm, vec![MockTool::new("search_symbols", "ok")], 5);
        let history = vec![
            HistoryMessage { role: "user".into(), text: "how does retry work?".into(), attachments: None },
            HistoryMessage { role: "assistant".into(), text: "The retry logic lives in retry.rs…".into(), attachments: None },
        ];
        let mode = agent.classify_intent("what does that mean?", &history, None).await;
        assert_eq!(mode, IntentMode::Conversational);
    }

    #[tokio::test]
    async fn query_streaming_emits_intent_event_first() {
        let llm = MockLlm::new(vec![
            text("research"),
            text("answer"),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("search_symbols", "ok")], 5));
        let events = collect_agent_events(agent, "how does X work?").await;
        assert!(matches!(events[0], AgentEvent::Intent { mode: IntentMode::Research }));
    }

    #[tokio::test]
    async fn conversational_mode_skips_tools_entirely() {
        let llm = MockLlm::new(vec![
            text("conversational"),
            text("direct answer"),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("search_symbols", "ok")], 5));
        let history = vec![
            HistoryMessage { role: "user".into(), text: "how does retry work?".into(), attachments: None },
            HistoryMessage { role: "assistant".into(), text: "The retry logic lives in retry.rs…".into(), attachments: None },
        ];
        let (tx, mut rx) = mpsc::channel::<AgentEvent>(128);
        agent.query_streaming("what does that mean?", &history, &[], None, tx).await;
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() { events.push(e); }
        let has_tool = events.iter().any(|e| matches!(e, AgentEvent::ToolCall { .. }));
        assert!(!has_tool, "conversational mode must not call any tools");
        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        });
        assert_eq!(answer.as_deref(), Some("direct answer"));
    }

    #[tokio::test]
    async fn multiple_tool_calls_in_one_turn_all_executed() {
        let llm = MockLlm::new(vec![
            text("research"),
            two_tool_calls("tool_a", "tool_b"),
            text("got both results"),
        ]);
        let agent = agent_with(
            llm,
            vec![MockTool::new("tool_a", "result_a"), MockTool::new("tool_b", "result_b")],
            5,
        );
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "got both results");
        assert_eq!(resp.tool_calls_made, 1);
    }

    #[tokio::test]
    async fn multi_turn_query_reports_provider_used_from_final_call() {
        let llm = MockLlm::with_id("mock-llm", vec![
            text("research"),
            tool_call("my_tool"),
            text("final answer"),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5);
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "final answer");
        let used = resp.provider_used.expect("expected provider_used to be set");
        assert_eq!(used.provider_id, "mock-llm");
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
    fn citation_line_range_parsed() {
        let sources = parse_citations("[r:v1:f.rs:328-335]");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].line, 328);
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
        let resp = agent.query("new question", &history, &[], None).await.unwrap();
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
        agent.query_streaming("new question", &history, &[], None, event_sender).await;
        let mut answer = None;
        while let Ok(event) = rx.try_recv() {
            if let AgentEvent::Done { answer: a, .. } = event {
                answer = Some(a);
            }
        }
        assert_eq!(answer.as_deref(), Some("streaming answer"));
    }

    struct CapturingLlm {
        response: Mutex<VecDeque<LlmResponse>>,
        captured_system: Mutex<Option<String>>,
    }

    impl CapturingLlm {
        fn new(responses: Vec<LlmResponse>) -> Arc<Self> {
            Arc::new(Self {
                response: Mutex::new(responses.into()),
                captured_system: Mutex::new(None),
            })
        }
    }

    #[async_trait]
    impl LlmProvider for CapturingLlm {
        fn id(&self) -> &str { "capturing-llm" }
        fn kind(&self) -> &str { "mock" }
        fn default_model(&self) -> &str { "mock-model" }
        async fn list_models(&self) -> Result<Vec<crate::llm::types::ModelInfo>> { Ok(vec![]) }

        async fn chat_with(&self, _model: Option<&str>, messages: &[Message], _tools: &[ToolDefinition]) -> Result<LlmResponse> {
            if let Some(first) = messages.first() {
                if matches!(first.role, crate::llm::types::Role::System) {
                    if let crate::llm::types::MessageContent::Text(t) = &first.content {
                        *self.captured_system.lock().unwrap() = Some(t.clone());
                    }
                }
            }
            self.response.lock().unwrap().pop_front()
                .ok_or_else(|| anyhow::anyhow!("CapturingLlm: no more responses"))
        }
    }

    #[tokio::test]
    async fn with_system_prompt_override_replaces_the_default_system_message() {
        let llm = CapturingLlm::new(vec![text("done")]);
        let agent = Arc::new(
            Agent::new(Arc::clone(&llm) as Arc<dyn LlmProvider>, vec![], 5)
                .with_system_prompt("You are a deployment assistant.".to_string()),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        agent.query_streaming("hi", &[], &[], None, tx).await;
        while rx.try_recv().is_ok() {}

        assert_eq!(llm.captured_system.lock().unwrap().as_deref(), Some("You are a deployment assistant."));
    }

    #[tokio::test]
    async fn without_override_uses_prompt_system_prompt() {
        let llm = CapturingLlm::new(vec![text("done")]);
        let agent = Arc::new(Agent::new(Arc::clone(&llm) as Arc<dyn LlmProvider>, vec![], 5));
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        agent.query_streaming("hi", &[], &[], None, tx).await;
        while rx.try_recv().is_ok() {}

        assert_eq!(llm.captured_system.lock().unwrap().as_deref(), Some(prompt::system_prompt().as_str()));
    }

    async fn collect_agent_events(agent: Arc<Agent>, query: &str) -> Vec<AgentEvent> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(128);
        agent.query_streaming(query, &[], &[], None, tx).await;
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
            text("research"),
            LlmResponse::ToolCalls {
                calls: vec![ToolCall { id: "t".into(), name: "my_tool".into(), input: serde_json::json!({}), thought_signature: None }],
                preamble: "Let me check that".into(),
            },
            text("done"),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("my_tool", "result")], 5));
        let events = collect_agent_events(agent, "hi").await;

        let preamble_pos = events.iter().position(|e| matches!(e, AgentEvent::TextDelta { text } if text == "Let me check that"))
            .expect("expected TextDelta with preamble text");
        let tool_pos = events.iter().position(|e| matches!(e, AgentEvent::ToolCall { name, .. } if name == "my_tool"))
            .expect("expected ToolCall event");
        assert!(preamble_pos < tool_pos, "preamble TextDelta must arrive before ToolCall");

        let has_thinking = events.iter().any(|e| matches!(e, AgentEvent::Thinking { .. }));
        assert!(!has_thinking, "no consolidated Thinking event — preamble streams live as TextDelta");
    }

    #[tokio::test]
    async fn no_preamble_emits_no_thinking_event_before_tool_call() {
        let llm = MockLlm::new(vec![
            text("research"),
            tool_call("my_tool"),
            text("done"),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5));
        let events = collect_agent_events(agent, "hi").await;

        let has_thinking = events.iter().any(|e| matches!(e, AgentEvent::Thinking { .. }));
        assert!(!has_thinking, "no synthesized Thinking event expected when preamble is empty");

        let tool_pos = events.iter().position(|e| matches!(e, AgentEvent::ToolCall { .. }))
            .expect("expected ToolCall event");

        let has_delta_before_tool = events[..tool_pos].iter().any(|e| matches!(e, AgentEvent::TextDelta { .. }));
        assert!(!has_delta_before_tool, "no TextDelta expected before tool call when preamble is empty");
    }

    #[tokio::test]
    async fn real_thinking_delta_suppresses_synthesized_thinking_event() {
        let llm = MockStreamingLlm::new(vec![
            vec![
                StreamEvent::ThinkingDelta { text: "Let me look at this closely".into() },
                StreamEvent::ToolCallReady(ToolCall {
                    id: "tc_1".into(), name: "my_tool".into(), input: serde_json::json!({}), thought_signature: None,
                }),
                StreamEvent::Done { stop_reason: "tool_use".into() },
            ],
            vec![
                StreamEvent::TextDelta { text: "done".into() },
                StreamEvent::Done { stop_reason: "end_turn".into() },
            ],
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5));
        let events = collect_agent_events(agent, "hi").await;

        let thinking_delta_pos = events.iter().position(|e| matches!(e, AgentEvent::ThinkingDelta { .. }))
            .expect("expected a streamed ThinkingDelta event");
        let tool_pos = events.iter().position(|e| matches!(e, AgentEvent::ToolCall { .. }))
            .expect("expected ToolCall event");
        assert!(thinking_delta_pos < tool_pos, "ThinkingDelta must precede ToolCall");

        let has_synthesized_thinking = events.iter().any(|e| matches!(e, AgentEvent::Thinking { .. }));
        assert!(!has_synthesized_thinking,
            "no synthesized Thinking event expected — real ThinkingDelta already streamed live");
    }

    #[tokio::test]
    async fn confirmable_tool_call_emits_confirm_action_and_ends_turn_without_executing() {
        let llm = MockLlm::new(vec![
            tool_call("delete_agent"),
        ]);
        let agent = Arc::new(agent_with(
            llm,
            vec![MockTool::new_confirmable("delete_agent", "should never run")],
            5,
        ));
        let events = collect_agent_events(agent, "delete the agent").await;

        let confirm = events.iter().find_map(|e| match e {
            AgentEvent::ConfirmAction { name, .. } => Some(name.clone()),
            _ => None,
        });
        assert_eq!(confirm.as_deref(), Some("delete_agent"));

        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::ToolCall { .. } | AgentEvent::ToolResult { .. })),
            "confirmable tool must not be executed or announced as a tool call before confirmation"
        );

        let done = events.iter().any(|e| matches!(e, AgentEvent::Done { .. }));
        assert!(done, "turn must end after requesting confirmation");
    }

    #[tokio::test]
    async fn confirm_action_carries_input_and_description() {
        let llm = MockLlm::new(vec![
            LlmResponse::ToolCalls {
                calls: vec![ToolCall { id: "tc_1".into(), name: "create_lxd_agent".into(), input: serde_json::json!({}), thought_signature: None }],
                preamble: "Provisioning a small container named build-runner".into(),
            },
        ]);
        let agent = Arc::new(agent_with(
            llm,
            vec![MockTool::new_confirmable("create_lxd_agent", "unused")],
            5,
        ));
        let events = collect_agent_events(agent, "create an agent").await;

        let (input, description) = events.iter().find_map(|e| match e {
            AgentEvent::ConfirmAction { input, description, .. } => Some((input.clone(), description.clone())),
            _ => None,
        }).expect("expected a ConfirmAction event");

        assert_eq!(input, serde_json::json!({}));
        assert_eq!(description, "Provisioning a small container named build-runner");
    }

    #[tokio::test]
    async fn multiple_confirmable_calls_in_one_round_all_pause_and_none_execute() {
        let llm = MockLlm::new(vec![
            text("research"),
            two_tool_calls("create_lxd_agent", "delete_agent"),
        ]);
        let agent = Arc::new(agent_with(
            llm,
            vec![
                MockTool::new_confirmable("create_lxd_agent", "unused"),
                MockTool::new_confirmable("delete_agent", "unused"),
            ],
            5,
        ));
        let events = collect_agent_events(agent, "do both").await;

        let confirm_names: Vec<String> = events.iter().filter_map(|e| match e {
            AgentEvent::ConfirmAction { name, .. } => Some(name.clone()),
            _ => None,
        }).collect();
        assert_eq!(confirm_names, vec!["create_lxd_agent".to_string(), "delete_agent".to_string()]);

        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::ToolCall { .. } | AgentEvent::ToolResult { .. })),
            "no confirmable call should execute before confirmation"
        );
    }

    #[tokio::test]
    async fn mixed_confirmable_and_automatic_calls_executes_automatic_and_pauses_confirmable() {
        let llm = MockLlm::new(vec![
            text("research"),
            two_tool_calls("my_tool", "delete_agent"),
        ]);
        let agent = Arc::new(agent_with(
            llm,
            vec![
                MockTool::new("my_tool", "ok"),
                MockTool::new_confirmable("delete_agent", "unused"),
            ],
            5,
        ));
        let events = collect_agent_events(agent, "do stuff").await;

        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolCall { name, .. } if name == "my_tool")));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolResult { name, .. } if name == "my_tool")));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ConfirmAction { name, .. } if name == "delete_agent")));
        assert!(!events.iter().any(|e| matches!(e, AgentEvent::ToolCall { name, .. } if name == "delete_agent")));
    }

    #[tokio::test]
    async fn resume_after_confirm_continues_the_loop_to_completion() {
        let llm = MockLlm::new(vec![
            tool_call("delete_agent"),
            text("Done, agent deleted."),
        ]);
        let agent = Arc::new(agent_with(
            llm,
            vec![MockTool::new_confirmable("delete_agent", "unused")],
            5,
        ));

        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        let paused = agent.query_streaming("delete it", &[], &[], None, tx).await
            .expect("expected the turn to pause");
        assert_eq!(paused.pending, vec![PendingConfirmCall { id: "tc_1:0".into(), tool_use_id: "tc_1".into() }]);
        while rx.try_recv().is_ok() {}

        let (tx2, mut rx2) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        let outcome = agent.resume_after_confirm(
            paused.messages,
            paused.iterations,
            vec![ToolResumeResult {
                tool_call_id: "tc_1".into(),
                content:      "Agent deleted.".into(),
                is_error:     false,
            }],
            None,
            tx2,
        ).await;
        assert!(outcome.is_none(), "expected the turn to finish, not pause again");

        let mut events = Vec::new();
        while let Ok(e) = rx2.try_recv() { events.push(e); }
        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        });
        assert_eq!(answer.as_deref(), Some("Done, agent deleted."));
    }

    #[tokio::test]
    async fn resume_after_confirm_can_pause_again_on_a_second_confirmable_call() {
        let llm = MockLlm::new(vec![
            text("research"),
            tool_call("create_lxd_agent"),
            tool_call("delete_agent"),
        ]);
        let agent = Arc::new(agent_with(
            llm,
            vec![
                MockTool::new_confirmable("create_lxd_agent", "unused"),
                MockTool::new_confirmable("delete_agent", "unused"),
            ],
            5,
        ));

        let (tx, _rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        let first = agent.query_streaming("do stuff", &[], &[], None, tx).await
            .expect("expected the first turn to pause");

        let (tx2, _rx2) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        let second = agent.resume_after_confirm(
            first.messages,
            first.iterations,
            vec![ToolResumeResult {
                tool_call_id: first.pending[0].tool_use_id.clone(),
                content:      "Agent created.".into(),
                is_error:     false,
            }],
            None,
            tx2,
        ).await.expect("expected the resumed turn to pause again");

        assert_eq!(second.pending, vec![PendingConfirmCall { id: "tc_1:0".into(), tool_use_id: "tc_1".into() }]);
    }

    #[tokio::test]
    async fn non_confirmable_tool_calls_execute_normally_alongside_confirmable_ones_absent() {
        let llm = MockLlm::new(vec![
            text("research"),
            tool_call("my_tool"),
            text("done"),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5));
        let events = collect_agent_events(agent, "hi").await;

        assert!(!events.iter().any(|e| matches!(e, AgentEvent::ConfirmAction { .. })));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolResult { .. })));
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

    // ── Fix 1: ask_user available in conversational mode ──

    #[tokio::test]
    async fn conversational_mode_ask_user_emits_question_event() {
        let llm = MockLlm::new(vec![
            text("conversational"),
            LlmResponse::ToolCalls {
                calls: vec![ToolCall {
                    id: "ask_1".into(),
                    name: "ask_user".into(),
                    input: serde_json::json!({"question": "Which aspect?", "choices": ["A", "B"]}),
                    thought_signature: None,
                }],
                preamble: String::new(),
            },
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("search_symbols", "ok")], 5));
        let events = collect_agent_events_with_history(agent, "what does that mean?", &[
            HistoryMessage { role: "user".into(), text: "how does X work?".into(), attachments: None },
            HistoryMessage { role: "assistant".into(), text: "X works like...".into(), attachments: None },
        ]).await;

        let has_question = events.iter().any(|e| matches!(e, AgentEvent::Question { question, .. } if question == "Which aspect?"));
        assert!(has_question, "conversational mode should emit Question event when ask_user is called");

        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        });
        assert!(answer.is_some(), "should have a Done event");
        assert!(!answer.as_deref().unwrap().contains("tool-call limit"),
            "conversational ask_user answer must not claim tool-call limit");
    }

    // ── Fix 2: extract_text_ask_user ──

    #[test]
    fn extract_text_ask_user_detects_json_fenced_block_with_question_key() {
        let input = "Some prose.\n```json\n{\"name\":\"ask_user\",\"arguments\":{\"question\":\"Q?\",\"choices\":[\"A\",\"B\"]}}\n```\nMore prose.";
        let (q, c, cleaned) = extract_text_ask_user(input).expect("should detect");
        assert_eq!(q, "Q?");
        assert_eq!(c, vec!["A", "B"]);
        assert!(cleaned.contains("Some prose."));
        assert!(cleaned.contains("More prose."));
        assert!(!cleaned.contains("ask_user"));
    }

    #[test]
    fn extract_text_ask_user_detects_bare_fenced_block_with_message_key() {
        let input = "```\n{\"name\":\"ask_user\",\"arguments\":{\"message\":\"Which one?\",\"choices\":[\"X\",\"Y\"]}}\n```";
        let (q, c, _) = extract_text_ask_user(input).expect("should detect");
        assert_eq!(q, "Which one?");
        assert_eq!(c, vec!["X", "Y"]);
    }

    #[test]
    fn extract_text_ask_user_filters_catchall_choices() {
        let input = "```json\n{\"name\":\"ask_user\",\"arguments\":{\"question\":\"Q?\",\"choices\":[\"A\",\"Other\",\"B\"]}}\n```";
        let (_, c, _) = extract_text_ask_user(input).expect("should detect");
        assert_eq!(c, vec!["A", "B"]);
    }

    #[test]
    fn extract_text_ask_user_returns_none_for_non_ask_user_json() {
        let input = "```json\n{\"name\":\"search_symbols\",\"arguments\":{\"query\":\"foo\"}}\n```";
        assert!(extract_text_ask_user(input).is_none());
    }

    #[test]
    fn extract_text_ask_user_returns_none_when_no_code_blocks() {
        assert!(extract_text_ask_user("just plain text").is_none());
    }

    #[test]
    fn extract_text_ask_user_returns_none_for_empty_question_or_choices() {
        let input = "```json\n{\"name\":\"ask_user\",\"arguments\":{\"question\":\"\",\"choices\":[\"A\"]}}\n```";
        assert!(extract_text_ask_user(input).is_none());
    }

    // ── Fix 3: use accumulated_text when text_buf is empty ──

    #[tokio::test]
    async fn ask_user_with_empty_text_buf_uses_accumulated_text() {
        let llm = MockLlm::new(vec![
            // Consumed by classify_intent
            text("research"),
            // Iteration 1: tool call WITH preamble text
            LlmResponse::ToolCalls {
                calls: vec![ToolCall {
                    id: "tc_1".into(),
                    name: "my_tool".into(),
                    input: serde_json::json!({}),
                    thought_signature: None,
                }],
                preamble: "I found some relevant code.".into(),
            },
            // Iteration 2: ask_user with NO preamble text
            LlmResponse::ToolCalls {
                calls: vec![ToolCall {
                    id: "ask_1".into(),
                    name: "ask_user".into(),
                    input: serde_json::json!({"question": "Which?", "choices": ["A", "B"]}),
                    thought_signature: None,
                }],
                preamble: String::new(),
            },
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5));
        let events = collect_agent_events(agent, "hi").await;

        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        }).expect("should have Done event");
        assert_eq!(answer, "I found some relevant code.", "answer should use accumulated_text from first turn");
    }

    // ── Fix 4: synthesis call when both text buffers empty ──

    #[tokio::test]
    async fn ask_user_with_no_text_anywhere_synthesizes_answer() {
        let llm = MockLlm::new(vec![
            // Consumed by classify_intent
            text("research"),
            // Iteration 1: tool call with no text preamble
            LlmResponse::ToolCalls {
                calls: vec![ToolCall {
                    id: "tc_1".into(),
                    name: "my_tool".into(),
                    input: serde_json::json!({}),
                    thought_signature: None,
                }],
                preamble: String::new(),
            },
            // Iteration 2: ask_user with no text preamble
            LlmResponse::ToolCalls {
                calls: vec![ToolCall {
                    id: "ask_1".into(),
                    name: "ask_user".into(),
                    input: serde_json::json!({"question": "Which?", "choices": ["A", "B"]}),
                    thought_signature: None,
                }],
                preamble: String::new(),
            },
            // Synthesis response (Fix 4)
            text("Here is what I found: the tool returned ok."),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5));
        let events = collect_agent_events(agent, "hi").await;

        let has_question = events.iter().any(|e| matches!(e, AgentEvent::Question { .. }));
        assert!(has_question, "should emit Question event");

        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        }).expect("should have Done event");
        assert!(answer.contains("Here is what I found"), "answer should use synthesized text, got: {answer}");
    }

    // ── Fix 5: question_fallback doesn't claim tool-call limit ──

    #[test]
    fn question_fallback_does_not_claim_tool_limit() {
        let fb = question_fallback();
        assert!(!fb.to_lowercase().contains("tool-call limit"));
        assert!(!fb.to_lowercase().contains("tool call limit"));
        assert!(!fb.is_empty());
    }

    #[test]
    fn last_resort_fallback_still_mentions_tool_limit() {
        let fb = last_resort_fallback();
        assert!(fb.to_lowercase().contains("tool-call limit"), "last_resort_fallback should still mention tool-call limit");
    }

    // ── Fix 6: synthesis ToolCalls with preamble uses preamble ──

    #[tokio::test]
    async fn max_iterations_synthesis_tool_calls_with_preamble_uses_preamble() {
        let llm = MockLlm::new(vec![
            LlmResponse::ToolCalls {
                calls: vec![ToolCall {
                    id: "tc_1".into(),
                    name: "my_tool".into(),
                    input: serde_json::json!({}),
                    thought_signature: None,
                }],
                preamble: String::new(),
            },
            // Synthesis returns ToolCalls with preamble
            LlmResponse::ToolCalls {
                calls: vec![],
                preamble: "Here's what I found from the tools.".into(),
            },
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "ok")], 1);
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "Here's what I found from the tools.");
    }

    // ── Helper for tests needing history ──

    async fn collect_agent_events_with_history(agent: Arc<Agent>, query: &str, history: &[HistoryMessage]) -> Vec<AgentEvent> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(128);
        agent.query_streaming(query, history, &[], None, tx).await;
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() { events.push(e); }
        events
    }
}

fn is_catchall(s: &str) -> bool {
    let lower = s.trim().to_lowercase();
    let stripped = lower.trim_end_matches(|c: char| matches!(c, '.' | '?' | '!') || c == '\u{2026}');
    matches!(stripped.trim(), "other" | "something else" | "none of the above" | "other option")
}

/// Detect an `ask_user` tool call that the LLM emitted as a JSON code block in
/// its text output instead of through the proper tool-calling mechanism.
///
/// Returns `(question, choices, cleaned_text)` where `cleaned_text` has the
/// matched code block removed. Handles both `"question"` and `"message"` as the
/// argument key (some models guess `"message"` when they can't see the schema).
fn extract_text_ask_user(text: &str) -> Option<(String, Vec<String>, String)> {
    let blocks = extract_code_blocks(text);
    for (block_content, span) in blocks {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&block_content) {
            let name = value.get("name").and_then(|v| v.as_str());
            if name != Some("ask_user") {
                continue;
            }
            let args = value.get("arguments").or(Some(&value))?;
            let question = args.get("question").or_else(|| args.get("message"))?
                .as_str()?
                .to_string();
            let choices: Vec<String> = args.get("choices")
                .and_then(|c| c.as_array())
                .map(|a| a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .filter(|s| !is_catchall(s))
                    .collect())
                .unwrap_or_default();
            if question.is_empty() || choices.is_empty() {
                continue;
            }
            let mut cleaned = String::with_capacity(text.len());
            cleaned.push_str(&text[..span.0]);
            cleaned.push_str(&text[span.1..]);
            let cleaned = cleaned.trim().to_string();
            return Some((question, choices, cleaned));
        }
    }
    None
}

/// Extract fenced code blocks from markdown text.
/// Returns `(block_content, (start, end))` where `start`/`end` are byte offsets
/// of the entire fence (including the ``` lines).
fn extract_code_blocks(text: &str) -> Vec<(String, (usize, usize))> {
    let mut blocks = Vec::new();
    let bytes = text.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        // Find the next ```
        let rest = &text[pos..];
        let Some(rel_open) = rest.find("```") else { break };
        let abs_open = pos + rel_open;
        // Skip optional language tag on the same line
        let line_end = text[abs_open..].find('\n')
            .map(|i| abs_open + i)
            .unwrap_or(text.len());
        let content_start = line_end + 1;
        // Find the closing ```
        if content_start >= text.len() {
            break;
        }
        let Some(rel_close) = text[content_start..].find("```") else { break };
        let abs_close = content_start + rel_close;
        // Find end of the closing fence line
        let fence_end = text[abs_close..].find('\n')
            .map(|i| abs_close + i + 1)
            .unwrap_or(text.len());
        let block_content = text[content_start..abs_close].trim().to_string();
        blocks.push((block_content, (abs_open, fence_end)));
        pos = fence_end;
    }
    blocks
}

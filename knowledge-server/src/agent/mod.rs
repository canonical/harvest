pub mod artifact_tools;
pub mod chain;
pub mod deployment_tools;
pub mod graph_tools;
pub mod lxd_tools;
pub mod machine_tools;
pub mod port_forward_tools;
pub mod skill_tools;
pub mod terraform_tools;
pub mod prompt;
pub mod semantic;
pub mod tool;

use anyhow::Result;
use futures::future::join_all;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::sync::Semaphore;

use crate::llm::{
    system_one::{SystemOneClient, ModelTier},
    types::{
        ContentPart, LlmResponse, Message, MessageContent, ProviderSelection, Role, StreamEvent,
        ToolCall, ToolDefinition, Usage, UsedProvider,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct QueryResponse {
    pub answer: String,
    pub sources: Vec<Source>,
    pub tool_calls_made: usize,
    pub tool_errors: usize,
    pub turns: usize,
    pub provider_used: Option<UsedProvider>,
    pub duration_ms: u64,
    pub usage: Usage,
    pub llm_call_count: usize,
    pub cost_microusd: i64,
}

impl QueryResponse {
    pub fn from_metrics(
        answer: String,
        sources: Vec<Source>,
        provider_used: Option<UsedProvider>,
        duration_ms: u64,
        llm_call_count: usize,
        metrics: &RunMetrics,
    ) -> Self {
        Self {
            answer,
            sources,
            tool_calls_made: metrics.tool_calls_executed,
            tool_errors: metrics.tool_errors,
            turns: metrics.turns,
            provider_used,
            duration_ms,
            usage: metrics.usage.clone(),
            llm_call_count,
            cost_microusd: 0,
        }
    }
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
        tool_errors: usize,
        turns: usize,
        provider_used: Option<UsedProvider>,
        duration_ms: u64,
        hit_max_iterations: bool,
        usage: Usage,
        llm_call_count: usize,
    },
    Error { message: String },
    Question { question: String, choices: Vec<String> },
    ConfirmAction { id: String, name: String, input: serde_json::Value, description: String },
    TitleUpdated { title: String },
    /// A durable marker that the agent split the question into independent
    /// leads and is investigating them concurrently. Unlike `Phase`, this is
    /// persisted into the visible chain instead of being a transient status
    /// label, so it survives to `Done` and to conversation reload.
    ParallelResearchStarted { leads: Vec<String> },
    /// Sent by each lead as it finishes (leads finish at different times).
    ParallelResearchLeadDone {
        index:       usize,
        iterations:  usize,
        preview:     String,
        duration_ms: u64,
    },
    /// Sent once all leads have completed and the merge step is about to
    /// resume on the visible loop. `duration_ms` is the wall-clock time of
    /// the fan-out (from ParallelResearchStarted to now) — the number to
    /// compare against a sequential-equivalent estimate.
    ParallelResearchMergeStarted { duration_ms: u64 },
}

enum LoopOutcome {
    Finished { text: String, iterations: usize, provider_used: Option<UsedProvider>, hit_max_iterations: bool, usage: Usage, llm_call_count: usize, tool_calls_executed: usize, tool_errors: usize },
    EndedWithQuestion { text: String, iterations: usize, provider_used: Option<UsedProvider>, usage: Usage, llm_call_count: usize, tool_calls_executed: usize, tool_errors: usize },
    Paused {
        messages: Vec<Message>,
        iterations: usize,
        text_buf: String,
        pending: Vec<PendingConfirmCall>,
        provider_used: Option<UsedProvider>,
        usage: Usage,
        llm_call_count: usize,
        tool_calls_executed: usize,
        tool_errors: usize,
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
    pub elapsed_ms: u64,
}

pub struct ToolResumeResult {
    pub tool_call_id: String,
    pub content:       String,
    pub is_error:      bool,
}

pub struct Agent {
    llm: Arc<dyn LlmProvider>,
    system_one: Option<Arc<SystemOneClient>>,
    tools: Vec<Box<dyn Tool>>,
    max_iterations: usize,
    compaction_threshold_chars: usize,
    compaction_keep_last: usize,
    system_prompt_override: Option<String>,
    enable_parallel_research: bool,
    llm_concurrency: Arc<Semaphore>,
    fast_path_threshold: f64,
    early_synthesis_threshold: f64,
    early_synthesis_research_threshold: f64,
    relevance_threshold: f64,
    relevance_preserve_recent: usize,
    early_synthesis_coverage_threshold: f64,
    early_synthesis_min_iterations_first_turn: usize,
    early_synthesis_uniform_threshold: f64,
    early_synthesis_capability_gate_threshold: f64,
    next_action_confidence: f64,
}

impl Agent {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        tools: Vec<Box<dyn Tool>>,
        max_iterations: usize,
    ) -> Self {
        Self {
            llm,
            system_one: None,
            tools,
            max_iterations,
            compaction_threshold_chars: usize::MAX,
            compaction_keep_last: 6,
            system_prompt_override: None,
            enable_parallel_research: false,
            llm_concurrency: Arc::new(Semaphore::new(20)),
            fast_path_threshold: 0.7,
            early_synthesis_threshold: 0.85,
            early_synthesis_research_threshold: 0.95,
            relevance_threshold: 0.4,
            relevance_preserve_recent: 2,
            early_synthesis_coverage_threshold: 0.8,
            early_synthesis_min_iterations_first_turn: 5,
            early_synthesis_uniform_threshold: 0.7,
            early_synthesis_capability_gate_threshold: 0.8,
            next_action_confidence: crate::llm::system_one::DEFAULT_NEXT_ACTION_CONFIDENCE,
        }
    }

    /// Offer `propose_parallel_research` to this agent. Only meaningful for
    /// an open-ended research/chat agent whose system prompt explains when to
    /// use it (see `prompt::system_prompt`'s "Parallel Research" section) —
    /// leave this off for a procedural task agent (design generation,
    /// provisioning, etc.) built with `with_system_prompt`: those prompts
    /// give the model a fixed sequence of steps to follow (write X, then
    /// call tool Y), and the tool's own description alone is enough to
    /// tempt a model into "splitting" that sequence into independent leads —
    /// silently dropping the very instruction to call Y, since the
    /// per-lead/merge prompts generated by that path know nothing about it.
    pub fn with_parallel_research(mut self, enabled: bool) -> Self {
        self.enable_parallel_research = enabled;
        self
    }

    pub fn with_system_one(mut self, client: Option<Arc<SystemOneClient>>) -> Self {
        self.system_one = client;
        self
    }

    pub fn with_thresholds(
        mut self,
        fast_path: f64,
        early_synthesis: f64,
        relevance: f64,
        early_synthesis_research: f64,
        relevance_preserve_recent: usize,
        early_synthesis_coverage: f64,
        early_synthesis_min_iterations_first_turn: usize,
        early_synthesis_uniform: f64,
    ) -> Self {
        self.fast_path_threshold = fast_path;
        self.early_synthesis_threshold = early_synthesis;
        self.relevance_threshold = relevance;
        self.early_synthesis_research_threshold = early_synthesis_research;
        self.relevance_preserve_recent = relevance_preserve_recent;
        self.early_synthesis_coverage_threshold = early_synthesis_coverage;
        self.early_synthesis_min_iterations_first_turn = early_synthesis_min_iterations_first_turn;
        self.early_synthesis_uniform_threshold = early_synthesis_uniform;
        self
    }

    pub fn with_capability_gate_threshold(mut self, threshold: f64) -> Self {
        self.early_synthesis_capability_gate_threshold = threshold;
        self
    }

    pub fn with_next_action_confidence(mut self, threshold: f64) -> Self {
        self.next_action_confidence = threshold;
        self
    }

    fn resolve_tier_selection(&self, tier: &ModelTier) -> Option<ProviderSelection> {
        let children = self.llm.children();
        if children.is_empty() {
            return None;
        }
        let providers: Vec<&Arc<dyn LlmProvider>> = match tier {
            ModelTier::Small => children.iter().last(),
            ModelTier::Medium => children.iter().nth(children.len() / 2),
            ModelTier::Large => children.iter().next(),
        }.into_iter().collect();
        providers.first().map(|p| ProviderSelection {
            provider_id: p.id().to_string(),
            model: None,
            cache_breakpoint_index: None,
        })
    }

    pub fn llm(&self) -> &Arc<dyn LlmProvider> {
        &self.llm
    }

    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt_override = Some(prompt);
        self
    }

    fn effective_system_prompt(&self) -> String {
        let collocate_enabled = self.tools.iter().any(|t| t.definition().name.starts_with("collocate_"));
        self.system_prompt_override.clone().unwrap_or_else(|| prompt::system_prompt(collocate_enabled))
    }

    async fn effective_system_prompt_for_query(&self, user_query: &str) -> String {
        if self.system_prompt_override.is_some() {
            return self.effective_system_prompt();
        }
        let collocate_enabled = self.tools.iter().any(|t| t.definition().name.starts_with("collocate_"));
        if let Some(so) = &self.system_one {
            match so.select_prompt_sections(user_query, collocate_enabled).await {
                Ok(flags) => {
                    prompt::system_prompt_with_sections(collocate_enabled, flags)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "system-one prompt section selection failed — using full prompt");
                    prompt::system_prompt(collocate_enabled)
                }
            }
        } else {
            prompt::system_prompt(collocate_enabled)
        }
    }

    pub fn with_compaction(mut self, threshold_chars: usize, keep_last: usize) -> Self {
        self.compaction_threshold_chars = threshold_chars;
        self.compaction_keep_last = keep_last;
        self
    }

    /// Cheap, free (no LLM call) classification used only to pick the first
    /// turn's tool set and give the UI an immediate "Intent" label. Anything
    /// this heuristic can't resolve defaults to `Research` with the full tool
    /// set attached — the model's own first response (a direct answer, a
    /// normal tool call, or a `propose_parallel_research` call) is what
    /// actually decides how the turn proceeds; see `run_loop`.
    pub async fn classify_intent(
        &self,
        user_query: &str,
        history: &[HistoryMessage],
        _selection: Option<&ProviderSelection>,
    ) -> IntentMode {
        if self.tools.is_empty() {
            return IntentMode::Conversational;
        }
        if let Some(so) = &self.system_one {
            let history_text = history
                .iter()
                .map(|m| format!("[{}]: {}", m.role, m.text))
                .collect::<Vec<_>>()
                .join("\n");
            match so.classify_intent(user_query, &history_text).await {
                Ok((mode, confidence)) if confidence >= 0.5 => return mode,
                Ok((_, low_conf)) => {
                    tracing::debug!(confidence = low_conf, "system-one intent low confidence — falling back to heuristic");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "system-one intent classification failed — falling back to heuristic");
                }
            }
        }
        self.classify_intent_heuristic(user_query, history)
    }

    fn classify_intent_heuristic(&self, user_query: &str, history: &[HistoryMessage]) -> IntentMode {
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
        IntentMode::default()
    }

    pub async fn compact_history(&self, history: &[HistoryMessage]) -> Vec<HistoryMessage> {
        if history.is_empty() || estimate_history_chars(history) <= self.compaction_threshold_chars {
            return history.to_vec();
        }
        if let Some(so) = &self.system_one {
            let history_text = history
                .iter()
                .map(|m| format!("[{}]: {}", m.role, m.text))
                .collect::<Vec<_>>()
                .join("\n");
            let current_query = history
                .last()
                .map(|m| m.text.as_str())
                .unwrap_or("");
            match so.should_compact(&history_text, current_query).await {
                Ok((should, _)) if !should => {
                    tracing::info!("system-one says compaction not needed — skipping");
                    return history.to_vec();
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "system-one compaction check failed — using char threshold");
                }
            }
        }
        self.compact_history_llm(history).await
    }

    async fn compact_history_llm(&self, history: &[HistoryMessage]) -> Vec<HistoryMessage> {
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
            Ok(LlmResponse::Message { text, .. }) => text,
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

    async fn compact_messages_mid_turn(&self, messages: Vec<Message>, protected_prefix_len: usize) -> Vec<Message> {
        let trace_len = messages.len().saturating_sub(protected_prefix_len);
        if trace_len <= MID_TURN_COMPACTION_KEEP_LAST {
            return messages;
        }
        if estimate_messages_chars(&messages[protected_prefix_len..]) <= MID_TURN_COMPACTION_CHAR_THRESHOLD {
            return messages;
        }
        if let Some(so) = &self.system_one {
            let trace_text = messages[protected_prefix_len..]
                .iter()
                .map(describe_message_for_compaction)
                .collect::<Vec<_>>()
                .join("\n");
            let current = messages
                .last()
                .map(|m| match &m.content {
                    MessageContent::Text(t) => t.as_str(),
                    MessageContent::Parts(_) => "",
                })
                .unwrap_or("");
            match so.should_compact(&trace_text, current).await {
                Ok((should, _)) if !should => {
                    tracing::info!("system-one says mid-turn compaction not needed — skipping");
                    return messages;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "system-one mid-turn compaction check failed — using char threshold");
                }
            }
        }
        self.compact_messages_mid_turn_llm(messages, protected_prefix_len).await
    }

    async fn compact_messages_mid_turn_llm(&self, messages: Vec<Message>, protected_prefix_len: usize) -> Vec<Message> {
        let trace_len = messages.len().saturating_sub(protected_prefix_len);

        let split_at = compaction_split_point(&messages, protected_prefix_len, protected_prefix_len + (trace_len - MID_TURN_COMPACTION_KEEP_LAST));
        if split_at <= protected_prefix_len {
            return messages;
        }
        let old = &messages[protected_prefix_len..split_at];
        let old_text = old.iter().map(describe_message_for_compaction).collect::<Vec<_>>().join("\n");
        let prompt = format!(
            "Summarize the following tool-use trace from an ongoing investigation. \
             Preserve concrete facts, file paths, function names, and findings. \
             This summary replaces the detailed trace as context for continuing the work.\n\n{old_text}"
        );

        let summary = match self.llm.chat(&[Message::user(prompt)], &[]).await {
            Ok(LlmResponse::Message { text, .. }) => text,
            _ => {
                tracing::warn!("mid-turn compaction LLM call failed — using full trace");
                return messages;
            }
        };

        tracing::info!(
            old_messages = old.len(),
            kept_messages = MID_TURN_COMPACTION_KEEP_LAST,
            "compacted mid-turn tool trace"
        );

        let mut result = Vec::with_capacity(protected_prefix_len + 1 + MID_TURN_COMPACTION_KEEP_LAST);
        result.extend_from_slice(&messages[..protected_prefix_len]);
        result.push(Message::user(format!("[Summary of earlier tool investigation]\n{summary}")));
        result.extend_from_slice(&messages[split_at..]);
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

        let stream_fut = self.query_streaming(user_query, history, attachments, selection, event_sender);
        let drain_fut = async {
            let mut response = None;
            let mut error = None;
            while let Some(event) = receiver.recv().await {
                match event {
                    AgentEvent::Done { answer, sources, tool_calls_made, tool_errors, turns, provider_used, duration_ms, usage, llm_call_count, .. } => {
                        response = Some(QueryResponse { answer, sources, tool_calls_made, tool_errors, turns, provider_used, duration_ms, usage, llm_call_count, cost_microusd: 0 });
                    }
                    AgentEvent::Error { message } => {
                        error = Some(anyhow::anyhow!(message));
                    }
                    _ => {}
                }
            }
            (response, error)
        };

        let (_paused, (response, error)) = tokio::join!(stream_fut, drain_fut);

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

        let stream_fut = self.query_streaming(user_query, history, attachments, selection, event_sender);
        let drain_fut = async {
            let mut response = None;
            let mut error = None;
            while let Some(event) = receiver.recv().await {
                let _ = progress.send(event.clone()).await;
                match event {
                    AgentEvent::Done { answer, sources, tool_calls_made, tool_errors, turns, provider_used, duration_ms, usage, llm_call_count, .. } => {
                        response = Some(QueryResponse { answer, sources, tool_calls_made, tool_errors, turns, provider_used, duration_ms, usage, llm_call_count, cost_microusd: 0 });
                    }
                    AgentEvent::Error { message } => {
                        error = Some(anyhow::anyhow!(message));
                    }
                    _ => {}
                }
            }
            (response, error)
        };

        let (_paused, (response, error)) = tokio::join!(stream_fut, drain_fut);

        response.ok_or_else(|| error.unwrap_or_else(|| anyhow::anyhow!("agent produced no response")))
    }

    fn build_tool_defs(&self) -> Vec<ToolDefinition> {
        let mut tool_defs: Vec<ToolDefinition> =
            self.tools.iter().map(|t| t.definition()).collect();
        tool_defs.push(ask_user_tool_def());
        if self.enable_parallel_research {
            tool_defs.push(propose_parallel_research_tool_def());
        }
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
        let start = Instant::now();
        let (mode, compacted) = tokio::join!(
            self.classify_intent(user_query, history, selection),
            self.compact_history(history),
        );
        let _ = event_sender.send(AgentEvent::Intent { mode }).await;

        let (tool_defs, tool_map) = match mode {
            IntentMode::Conversational => (vec![ask_user_tool_def()], HashMap::new()),
            _ => (self.build_tool_defs(), self.build_tool_map()),
        };

        let system_prompt = self.effective_system_prompt_for_query(user_query).await;
        let mut messages = vec![Message::system(system_prompt)];
        messages.extend(history_to_messages(&compacted));
        messages.push(build_user_message(user_query, attachments));

        let is_first_turn = history.is_empty();
        let outcome = self.run_loop(messages, 0, &tool_defs, &tool_map, selection, &event_sender, self.max_iterations, 0, mode, is_first_turn).await;
        self.finish_outcome(outcome, &event_sender, start, 0).await
    }

    pub async fn resume_after_confirm(
        &self,
        mut messages: Vec<Message>,
        iterations: usize,
        results: Vec<ToolResumeResult>,
        selection: Option<&ProviderSelection>,
        event_sender: mpsc::Sender<AgentEvent>,
        elapsed_before_ms: u64,
    ) -> Option<PausedTurn> {
        let start = Instant::now();
        for r in results {
            messages.push(Message {
                role: crate::llm::types::Role::User,
                content: MessageContent::Parts(vec![ContentPart::ToolResult {
                    tool_use_id: r.tool_call_id,
                    content:     cap_tool_result(r.content),
                    is_error:    r.is_error,
                }]),
            });
        }

        let tool_defs = self.build_tool_defs();
        let tool_map  = self.build_tool_map();

        let outcome = self.run_loop(messages, iterations, &tool_defs, &tool_map, selection, &event_sender, self.max_iterations, 0, IntentMode::Research, false).await;
        self.finish_outcome(outcome, &event_sender, start, elapsed_before_ms).await
    }

    async fn finish_outcome(
        &self,
        outcome: LoopOutcome,
        event_sender: &mpsc::Sender<AgentEvent>,
        start: Instant,
        elapsed_before_ms: u64,
    ) -> Option<PausedTurn> {
        let duration_ms = elapsed_before_ms + start.elapsed().as_millis() as u64;
        let (turns, llm_calls, usage, tool_calls_executed, tool_errors) = match &outcome {
            LoopOutcome::Finished { iterations, llm_call_count, usage, tool_calls_executed, tool_errors, .. } => (*iterations, *llm_call_count, usage.clone(), *tool_calls_executed, *tool_errors),
            LoopOutcome::EndedWithQuestion { iterations, llm_call_count, usage, tool_calls_executed, tool_errors, .. } => (*iterations, *llm_call_count, usage.clone(), *tool_calls_executed, *tool_errors),
            LoopOutcome::Paused { iterations, llm_call_count, usage, tool_calls_executed, tool_errors, .. } => (*iterations, *llm_call_count, usage.clone(), *tool_calls_executed, *tool_errors),
        };
        let metrics = RunMetrics { tool_calls_executed, tool_errors, turns, llm_calls, duration_ms, usage };
        tracing::info!(
            turns = metrics.turns,
            llm_calls = metrics.llm_calls,
            tool_calls_executed = metrics.tool_calls_executed,
            tool_errors = metrics.tool_errors,
            input_tokens = metrics.usage.input_tokens,
            output_tokens = metrics.usage.output_tokens,
            cache_read_tokens = metrics.usage.cache_read_tokens,
            duration_ms = metrics.duration_ms,
            "agent run metrics"
        );
        match outcome {
            LoopOutcome::Finished { text, iterations, provider_used, hit_max_iterations, usage, llm_call_count, .. } => {
                let answer = if text.trim().is_empty() {
                    let reason = if hit_max_iterations { IncompleteReason::ToolLimit } else { IncompleteReason::EmptyReply };
                    incomplete_answer(&reason)
                } else {
                    strip_answer_preamble(&text)
                };
                let sources = parse_citations(&answer);
                let _ = event_sender.send(AgentEvent::Done {
                    answer,
                    sources,
                    tool_calls_made: metrics.tool_calls_executed,
                    tool_errors: metrics.tool_errors,
                    turns: metrics.turns,
                    provider_used,
                    duration_ms,
                    hit_max_iterations,
                    usage,
                    llm_call_count,
                }).await;
                None
            }
            LoopOutcome::EndedWithQuestion { text, iterations, provider_used, usage, llm_call_count, .. } => {
                let answer = if text.is_empty() { question_fallback() } else { text };
                let sources = parse_citations(&answer);
                let _ = event_sender.send(AgentEvent::Done {
                    answer,
                    sources,
                    tool_calls_made: metrics.tool_calls_executed,
                    tool_errors: metrics.tool_errors,
                    turns: metrics.turns,
                    provider_used,
                    duration_ms,
                    hit_max_iterations: false,
                    usage,
                    llm_call_count,
                }).await;
                None
            }
            LoopOutcome::Paused { messages, iterations, text_buf, pending, provider_used, usage, llm_call_count, .. } => {
                let answer = if text_buf.is_empty() { question_fallback() } else { text_buf };
                let _ = event_sender.send(AgentEvent::Done {
                    answer,
                    sources: vec![],
                    tool_calls_made: metrics.tool_calls_executed,
                    tool_errors: metrics.tool_errors,
                    turns: metrics.turns,
                    provider_used,
                    duration_ms,
                    hit_max_iterations: false,
                    usage,
                    llm_call_count,
                }).await;
                Some(PausedTurn { messages, iterations, pending, elapsed_ms: duration_ms })
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
        max_iterations: usize,
        depth: usize,
        intent: IntentMode,
        is_first_turn: bool,
    ) -> LoopOutcome {
        let mut last_provider_used: Option<UsedProvider> = None;
        let mut accumulated_text = String::new();
        let mut accumulated_answer = String::new();
        let mut consecutive_searches: usize = 0;
        let mut total_usage = Usage::default();
        let mut llm_call_count: usize = 0;
        let protected_prefix_len = messages.len();
        let goal = latest_user_text(&messages);
        let mut scored_tool_results: HashSet<u64> = HashSet::new();
        loop {
            if iterations >= max_iterations {
                tracing::warn!(max_iterations, "agent hit max_iterations — requesting synthesis");
                let _ = event_sender.send(AgentEvent::Phase { label: "Synthesizing answer".to_string() }).await;
                let tool_summary = collect_tool_result_summary(&messages);
                let is_uniform = match &self.system_one {
                    Some(so) if intent == IntentMode::Research => {
                        match so.should_synthesize_uniform(&goal, &tool_summary).await {
                            Ok((uniform, _)) => uniform,
                            Err(e) => {
                                tracing::warn!(error = %e, "uniformity check failed at max_iterations — assuming non-uniform");
                                false
                            }
                        }
                    }
                    _ => false,
                };
                let synthesis_prompt = build_synthesis_prompt(
                    "You have used the maximum number of tool calls. Synthesize what you have gathered so far into a final answer.",
                    &tool_summary,
                    is_uniform,
                );
                let synthesis = self.synthesize(&mut messages, synthesis_prompt, selection, &mut last_provider_used, &mut total_usage, &mut llm_call_count).await;
                let text = synthesis.into_answer(&accumulated_answer, &accumulated_text, IncompleteReason::ToolLimit);
                return LoopOutcome::Finished { text, iterations, provider_used: last_provider_used, hit_max_iterations: true, usage: total_usage, llm_call_count, tool_calls_executed: tally_count(&messages), tool_errors: tally_errors(&messages) };
            }

            let (stream_tx, mut stream_rx) = mpsc::channel::<StreamEvent>(64);
            let llm            = Arc::clone(&self.llm);
            let msgs_snapshot  = messages.clone();
            let mut selection_owned = selection.cloned();
            if let Some(sel) = &mut selection_owned {
                sel.cache_breakpoint_index = Some(protected_prefix_len);
            }

            let history_summary: String = messages.iter()
                .take(messages.len().saturating_sub(1))
                .filter(|m| matches!(m.role, crate::llm::types::Role::User | crate::llm::types::Role::Assistant))
                .map(|m| match &m.content {
                    MessageContent::Text(t) => t.clone(),
                    MessageContent::Parts(_) => String::new(),
                })
                .collect::<Vec<_>>()
                .join("\n");

            let tools_snapshot = if let Some(so) = &self.system_one {
                let all_tool_names: Vec<String> = tool_defs.iter().map(|t| t.name.clone()).collect();
                match so.select_tools(&goal, &all_tool_names).await {
                    Ok(selected) => {
                        tool_defs.iter()
                            .filter(|t| selected.contains(&t.name) || t.name == "ask_user")
                            .cloned()
                            .collect()
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "system-one tool selection failed — using full tool set");
                        tool_defs.to_vec()
                    }
                }
            } else {
                tool_defs.to_vec()
            };

            if let Some(so) = &self.system_one {
                if selection_owned.is_none() {
                    match so.route_model_enhanced(&goal, &history_summary).await {
                        Ok((tier, confidence, _needs_deep)) if confidence >= self.fast_path_threshold => {
                            selection_owned = self.resolve_tier_selection(&tier);
                        }
                        Ok((_, low_conf, _)) => {
                            tracing::debug!(confidence = low_conf, "system-one model routing low confidence — using default");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "system-one model routing failed — using default provider");
                        }
                    }
                }
            }

            let permit = self.llm_concurrency.clone().acquire_owned().await;
            let stream_handle = tokio::spawn(async move {
                let _permit = permit;
                llm.chat_stream_routed(selection_owned.as_ref(), &msgs_snapshot, &tools_snapshot, stream_tx).await
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
                        let _ = event_sender.send(AgentEvent::TextDelta { text: text.clone() }).await;
                        text_buf.push_str(&text);
                    }
                    StreamEvent::ToolCallReady(call) => {
                        tool_calls.push(call);
                    }
                    StreamEvent::Done { stop_reason: sr, usage } => {
                        stop_reason = sr;
                        total_usage += usage;
                        llm_call_count += 1;
                    }
                }
            }

            let stream_error: Option<String> = match stream_handle.await {
                Ok(Ok(used)) => {
                    last_provider_used = Some(used);
                    None
                }
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "chat_stream failed");
                    Some(e.to_string())
                }
                Err(e) => {
                    tracing::warn!(error = %e, "chat_stream task panicked");
                    Some(format!("the model request task failed: {e}"))
                }
            };

            if !text_buf.is_empty() {
                accumulated_text.push_str(&text_buf);
            }

            if stop_reason == "max_tokens" && tool_calls.is_empty() && iterations + 1 < max_iterations {
                tracing::warn!(iteration = iterations, chars = text_buf.len(), "LLM hit max_tokens — auto-continuing");
                let _ = event_sender.send(AgentEvent::Phase { label: "Continuing…".to_string() }).await;
                accumulated_answer.push_str(&text_buf);
                messages.push(Message::assistant_text(text_buf));
                messages.push(Message::user("Continue from exactly where you left off. Do not repeat any content already written — start mid-sentence if necessary and complete the document."));
                iterations += 1;
                continue;
            }

            if stop_reason == "end_turn" || tool_calls.is_empty() {
                if let Some((question, choices, cleaned)) = extract_text_ask_user(&text_buf) {
                    let answer_text = self.resolve_ask_user_answer_text(
                        cleaned, &messages, selection, &accumulated_text, &question,
                    ).await;
                    let _ = event_sender.send(AgentEvent::Question { question, choices }).await;
                    return LoopOutcome::EndedWithQuestion {
                        text: answer_text, iterations, provider_used: last_provider_used,
                        usage: total_usage.clone(), llm_call_count, tool_calls_executed: tally_count(&messages), tool_errors: tally_errors(&messages)
                    };
                }
                let mut final_text = if accumulated_answer.is_empty() {
                    text_buf
                } else {
                    accumulated_answer.push_str(&text_buf);
                    accumulated_answer
                };
                if final_text.trim().is_empty() {
                    // The model failed or replied with nothing (reasoning models can spend the
                    // whole reply on hidden reasoning). Ask once more for an answer, without tools,
                    // from what was already gathered rather than ending on an empty message.
                    let reason = match stream_error {
                        Some(e) => IncompleteReason::ModelError(e),
                        None => IncompleteReason::EmptyReply,
                    };
                    tracing::warn!(?reason, iteration = iterations, "model produced no answer text — attempting recovery synthesis");
                    final_text = if tally_count(&messages) > 0 {
                        let _ = event_sender.send(AgentEvent::Phase { label: "Synthesizing answer".to_string() }).await;
                        let prompt = build_synthesis_prompt(
                            "Write your final answer to the user's question now, using only the tool results you already have.",
                            &collect_tool_result_summary(&messages),
                            false,
                        );
                        let synthesis = self.synthesize(&mut messages, prompt, selection, &mut last_provider_used, &mut total_usage, &mut llm_call_count).await;
                        synthesis.into_answer("", "", reason)
                    } else {
                        incomplete_answer(&reason)
                    };
                } else if let Some(e) = stream_error {
                    final_text.push_str(&format!("\n\n_The response was cut off because the model request failed: {}_", short_error(&e)));
                }
                return LoopOutcome::Finished {
                    text: final_text,
                    iterations,
                    provider_used: last_provider_used,
                    hit_max_iterations: false,
                    usage: total_usage.clone(),
                    llm_call_count, tool_calls_executed: tally_count(&messages), tool_errors: tally_errors(&messages)
                };
            }

            iterations += 1;

            if let Some(propose) = tool_calls.iter().find(|c| c.name == "propose_parallel_research") {
                let leads: Vec<String> = propose.input["leads"].as_array()
                    .map(|a| a.iter()
                        .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from))
                        .take(MAX_PARALLEL_LEADS)
                        .collect())
                    .unwrap_or_default();
                if leads.len() >= 2 {
                    let should_proceed = if let Some(so) = &self.system_one {
                        let history_text = messages.iter()
                            .filter(|m| matches!(m.role, crate::llm::types::Role::User | crate::llm::types::Role::Assistant))
                            .map(|m| match &m.content {
                                MessageContent::Text(t) => t.clone(),
                                MessageContent::Parts(_) => String::new(),
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        match so.should_parallel_research(&goal, &history_text).await {
                            Ok((should, _)) => should,
                            Err(e) => {
                                tracing::warn!(error = %e, "system-one parallel research gate failed — allowing");
                                true
                            }
                        }
                    } else {
                        true
                    };
                    if should_proceed {
                        return Box::pin(self.run_parallel_research(
                            messages, leads, tool_defs, tool_map, selection, event_sender, depth + 1,
                        )).await;
                    }
                }
            }

            if let Some(ask) = tool_calls.iter().find(|c| c.name == "ask_user") {
                let question = ask.input["question"].as_str().unwrap_or("").to_string();
                let choices = ask.input["choices"]
                    .as_array()
                    .map(|a| a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .filter(|s| !is_catchall(s))
                        .collect())
                    .unwrap_or_default();
                let answer_text = self.resolve_ask_user_answer_text(
                    text_buf, &messages, selection, &accumulated_text, &question,
                ).await;
                let _ = event_sender.send(AgentEvent::Question { question, choices }).await;
                return LoopOutcome::EndedWithQuestion {
                    text: answer_text, iterations, provider_used: last_provider_used,
                    usage: total_usage.clone(), llm_call_count, tool_calls_executed: tally_count(&messages), tool_errors: tally_errors(&messages)
                };
            }

            let tool_calls = if let Some(so) = &self.system_one {
                if tool_calls.is_empty() {
                    tool_calls
                } else {
                    let history_text = messages.iter()
                        .filter(|m| matches!(m.role, crate::llm::types::Role::User | crate::llm::types::Role::Assistant))
                        .map(|m| match &m.content {
                            MessageContent::Text(t) => t.clone(),
                            MessageContent::Parts(_) => String::new(),
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let call_infos: Vec<(String, String)> = tool_calls.iter()
                        .map(|c| (c.name.clone(), c.input.to_string()))
                        .collect();
                    match so.filter_tool_calls(&call_infos, &history_text).await {
                        Ok(keep) => {
                            let filtered: Vec<ToolCall> = tool_calls.into_iter()
                                .zip(keep.iter())
                                .filter(|(_, k)| **k)
                                .map(|(c, _)| c)
                                .collect();
                            if filtered.len() < call_infos.len() {
                                tracing::info!(
                                    total = call_infos.len(),
                                    kept = filtered.len(),
                                    "system-one filtered unnecessary tool calls"
                                );
                            }
                            filtered
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "system-one tool call filtering failed — executing all");
                            tool_calls
                        }
                    }
                }
            } else {
                tool_calls
            };

            if tool_calls.is_empty() {
                let final_text = if accumulated_answer.is_empty() {
                    text_buf
                } else {
                    accumulated_answer
                };
                return LoopOutcome::Finished {
                    text: final_text,
                    iterations,
                    provider_used: last_provider_used,
                    hit_max_iterations: false,
                    usage: total_usage.clone(),
                    llm_call_count, tool_calls_executed: tally_count(&messages), tool_errors: tally_errors(&messages)
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
                                content:     cap_tool_result(result),
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
                let t = (tally_count(&messages), tally_errors(&messages));
                return LoopOutcome::Paused { messages, iterations, text_buf: paused_text, pending, provider_used: last_provider_used, usage: total_usage.clone(), llm_call_count, tool_calls_executed: t.0, tool_errors: t.1 };
            }

            let phase = derive_phase(&tool_calls);
            let _ = event_sender.send(AgentEvent::Phase { label: phase.to_string() }).await;

            let has_search = tool_calls.iter().any(|c| c.name == "search_symbols");
            let has_deep = tool_calls.iter().any(|c| is_deep_read_tool(&c.name));
            if has_search && !has_deep {
                consecutive_searches += 1;
            } else if has_deep {
                consecutive_searches = 0;
            }
            let nudge_after_results = consecutive_searches >= 3;
            if nudge_after_results {
                consecutive_searches = 0;
            }

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
                        content:     cap_tool_result(result),
                        is_error:    false,
                    }]),
                });
            }
            if nudge_after_results {
                // Attached to the tool result rather than sent as a separate user message: a user
                // message between a tool call and its result breaks the provider message order,
                // and the model reads a user-role nudge as the user speaking.
                append_to_last_tool_result(&mut messages, SEARCH_NUDGE);
            }

            if let Some(so) = &self.system_one {
                let preserve_recent = self.relevance_preserve_recent;
                let cutoff = messages.len().saturating_sub(preserve_recent);
                let tool_names = tool_names_by_call_id(&messages);
                for i in (protected_prefix_len..cutoff).rev() {
                    let Some((tool_use_id, result_content)) = first_tool_result(&messages[i]) else { continue };
                    // Keyed on content as well as id: Gemini reuses the tool name as the call id.
                    if !scored_tool_results.insert(relevance_key(&tool_use_id, &result_content)) {
                        continue;
                    }
                    let tool_name = tool_names.get(&tool_use_id).map(String::as_str).unwrap_or("unknown");
                    match so.score_tool_result_relevance(tool_name, &result_content, &goal).await {
                        Ok(score) if score < self.relevance_threshold => {
                            let head: String = result_content.chars().take(200).collect();
                            let truncated = format!("[Truncated — relevance {score:.2}]\n{head}…");
                            if let MessageContent::Parts(parts) = &mut messages[i].content {
                                for part in parts.iter_mut() {
                                    if let ContentPart::ToolResult { content, .. } = part {
                                        *content = truncated;
                                        break;
                                    }
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(error = %e, "relevance scoring failed — keeping full result"),
                    }
                }

                let min_iterations = if is_first_turn && intent == IntentMode::Research {
                    self.early_synthesis_min_iterations_first_turn
                } else {
                    3
                };
                if iterations >= min_iterations {
                    let findings = collect_tool_result_summary(&messages);
                    let mut next_action_vetoes_synthesis: Option<bool> = None;
                    if let Some(so) = &self.system_one {
                        let candidates = [
                            crate::llm::system_one::NextAction::GetEvidencePack,
                            crate::llm::system_one::NextAction::ReadSources,
                            crate::llm::system_one::NextAction::SearchSymbols,
                            crate::llm::system_one::NextAction::Synthesize,
                        ];
                        match so
                            .route_next_action(&goal, &findings, &candidates, self.next_action_confidence)
                            .await
                        {
                            Ok(decision) if !decision.fallback => {
                                let vetoes = decision.action != crate::llm::system_one::NextAction::Synthesize;
                                tracing::debug!(
                                    action = decision.action.key(),
                                    confidence = decision.confidence,
                                    vetoes_synthesis = vetoes,
                                    "system-one routed the next action"
                                );
                                next_action_vetoes_synthesis = Some(vetoes);
                            }
                            Ok(decision) => {
                                tracing::debug!(
                                    action = decision.action.key(),
                                    confidence = decision.confidence,
                                    "next-action routing was below the confidence threshold"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "next-action routing failed — keeping the existing synthesis gate");
                            }
                        }
                    }
                    let synth_threshold = if intent == IntentMode::Research {
                        self.early_synthesis_research_threshold
                    } else {
                        self.early_synthesis_threshold
                    };
                    let should_synthesize_now: Option<bool> = match so.should_synthesize(&goal, &findings).await {
                        Ok((should, confidence)) if should && confidence >= synth_threshold => {
                            if intent == IntentMode::Research {
                                match so.should_synthesize_coverage(&goal, &findings).await {
                                    Ok((covered, cov_conf)) if covered && cov_conf >= self.early_synthesis_coverage_threshold => {
                                        match so.should_synthesize_capability_gate_with_threshold(
                                            &goal,
                                            &findings,
                                            self.early_synthesis_capability_gate_threshold,
                                        ).await {
                                            Ok((gate_located, gate_conf)) if !gate_located => {
                                                tracing::debug!(gate_conf, "early synthesis deferred — capability gate not located");
                                                None
                                            }
                                            Ok(_) => {
                                                if next_action_vetoes_synthesis == Some(true) {
                                                    tracing::debug!("early synthesis deferred — next-action router still wants more evidence");
                                                    None
                                                } else {
                                                    match so.should_synthesize_uniform(&goal, &findings).await {
                                                        Ok((uniform, uni_conf)) => Some(uniform && uni_conf >= self.early_synthesis_uniform_threshold),
                                                        Err(e) => {
                                                            tracing::warn!(error = %e, "uniformity check failed — assuming non-uniform");
                                                            Some(false)
                                                        }
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                tracing::warn!(error = %e, "capability gate check failed — assuming located");
                                                match so.should_synthesize_uniform(&goal, &findings).await {
                                                    Ok((uniform, uni_conf)) => Some(uniform && uni_conf >= self.early_synthesis_uniform_threshold),
                                                    Err(e) => {
                                                        tracing::warn!(error = %e, "uniformity check failed — assuming non-uniform");
                                                        Some(false)
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Ok((_, cov_conf)) => {
                                        tracing::debug!(cov_conf, "early synthesis deferred — coverage incomplete");
                                        None
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "early synthesis coverage check failed");
                                        None
                                    }
                                }
                            } else {
                                Some(true)
                            }
                        }
                        Ok(_) => None,
                        Err(e) => {
                            tracing::warn!(error = %e, "early synthesis check failed");
                            None
                        }
                    };
                    if let Some(is_uniform) = should_synthesize_now {
                        tracing::info!(iterations, intent = ?intent, "system-one triggered early synthesis");
                        let _ = event_sender.send(AgentEvent::Phase { label: "Synthesizing answer".to_string() }).await;
                        let synthesis_prompt = build_synthesis_prompt(
                            "Based on your investigation so far, provide a complete answer to the user's question.",
                            &findings,
                            is_uniform,
                        );
                        let synthesis = self.synthesize(&mut messages, synthesis_prompt, selection, &mut last_provider_used, &mut total_usage, &mut llm_call_count).await;
                        let text = synthesis.into_answer(&accumulated_answer, &accumulated_text, IncompleteReason::EmptyReply);
                        return LoopOutcome::Finished {
                            text, iterations,
                            provider_used: last_provider_used,
                            hit_max_iterations: false,
                            usage: total_usage, llm_call_count, tool_calls_executed: tally_count(&messages), tool_errors: tally_errors(&messages)
                        };
                    }
                }
            }

            messages = self.compact_messages_mid_turn(messages, protected_prefix_len).await;
        }
    }

    async fn run_parallel_research(
        &self,
        base_messages: Vec<Message>,
        subtasks: Vec<String>,
        tool_defs: &[ToolDefinition],
        tool_map: &HashMap<String, &dyn Tool>,
        selection: Option<&ProviderSelection>,
        event_sender: &mpsc::Sender<AgentEvent>,
        depth: usize,
    ) -> LoopOutcome {
        let fan_out_start = Instant::now();
        let _ = event_sender.send(AgentEvent::ParallelResearchStarted { leads: subtasks.clone() }).await;

        let sub_tool_map: HashMap<String, &dyn Tool> = tool_map.iter()
            .filter(|(_, t)| !t.requires_confirmation())
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let mut sub_tool_defs: Vec<ToolDefinition> = tool_defs.iter()
            .filter(|t| sub_tool_map.contains_key(&t.name))
            .cloned()
            .collect();
        if depth < MAX_PARALLEL_RESEARCH_DEPTH {
            sub_tool_defs.push(propose_parallel_research_tool_def());
        }
        let sub_max_iterations = (self.max_iterations / 2).clamp(3, 8);

        let leads = join_all(subtasks.iter().enumerate().map(|(index, subtask)| {
            let mut sub_messages = base_messages.clone();
            sub_messages.push(Message::user(format!(
                "Investigate ONLY the following specific question using the available tools, \
                 then report your findings with citations. Since your findings will be compared \
                 side by side with other independent leads, cover the same kind of ground they \
                 likely will: how it's configured or triggered, the end-to-end flow (describe \
                 the steps in order if it's a process), and notable edge cases such as \
                 first-time/new-record handling or error handling. Do not ask the user \
                 anything — if information is missing or ambiguous, give your best answer from \
                 what the graph contains.\n\nQuestion: {subtask}"
            )));
            self.run_research_subtask(
                index, sub_messages, &sub_tool_defs, &sub_tool_map, sub_max_iterations, selection, depth,
                event_sender,
            )
        })).await;

        let total_iterations: usize = leads.iter().map(|(_, iters, _, _)| iters).sum();
        let sub_usage: Usage = leads.iter().map(|(_, _, u, _)| u.clone()).fold(Usage::default(), |a, b| a + b);
        let sub_call_count: usize = leads.iter().map(|(_, _, _, c)| *c).sum();
        let findings = subtasks.iter().zip(leads.iter())
            .map(|(subtask, (text, _, _, _))| format!("### Lead: {subtask}\n{text}"))
            .collect::<Vec<_>>()
            .join("\n\n");

        let fan_out_ms = fan_out_start.elapsed().as_millis() as u64;
        tracing::info!(
            leads = subtasks.len(),
            total_lead_iterations = total_iterations,
            wall_ms = fan_out_ms,
            "parallel research fan-out complete"
        );
        let _ = event_sender.send(AgentEvent::ParallelResearchMergeStarted { duration_ms: fan_out_ms }).await;

        let mut messages = base_messages;
        messages.push(Message::user(format!(
            "You split the user's question into {} independent leads and investigated each — \
             their findings are below. Using ONLY these findings (call more tools only if \
             something essential is still missing), write ONE final answer to the user's \
             original question.\n\n\
             Before writing, check whether the leads cover comparable ground — if one lead's \
             findings address a dimension (configuration, end-to-end flow, edge cases like \
             new-record or error handling) that another lead's findings do not, and it matters \
             for a fair comparison, call a tool now to fill that specific gap rather than \
             leaving the comparison lopsided. You may make at most 2 such gap-filling calls; do \
             not re-investigate everything from scratch. If a flow was described step by step in \
             the findings, prefer a sequence diagram over a single relationship graph to show \
             it.\n\n\
             Preserve inline citations from the findings verbatim, reconcile any \
             inconsistencies between leads, and do not repeat the same fact twice.\n\n{findings}",
            subtasks.len(),
        )));

        let start_at = total_iterations.min(self.max_iterations.saturating_sub(1));
        let mut outcome = self.run_loop(messages, start_at, tool_defs, tool_map, selection, event_sender, self.max_iterations, depth, IntentMode::Research, false).await;
        match &mut outcome {
            LoopOutcome::Finished { usage, llm_call_count, .. } => {
                *usage = std::mem::take(usage) + sub_usage.clone();
                *llm_call_count += sub_call_count;
            }
            LoopOutcome::EndedWithQuestion { usage, llm_call_count, .. } => {
                *usage = std::mem::take(usage) + sub_usage.clone();
                *llm_call_count += sub_call_count;
            }
            LoopOutcome::Paused { usage, llm_call_count, .. } => {
                *usage = std::mem::take(usage) + sub_usage.clone();
                *llm_call_count += sub_call_count;
            }
        }
        outcome
    }

    async fn run_research_subtask(
        &self,
        index: usize,
        messages: Vec<Message>,
        tool_defs: &[ToolDefinition],
        tool_map: &HashMap<String, &dyn Tool>,
        max_iterations: usize,
        selection: Option<&ProviderSelection>,
        depth: usize,
        event_sender: &mpsc::Sender<AgentEvent>,
    ) -> (String, usize, Usage, usize) {
        let lead_start = Instant::now();
        let (sub_tx, mut sub_rx) = mpsc::channel::<AgentEvent>(64);
        let sender_clone = event_sender.clone();
        let forward_handle = tokio::spawn(async move {
            while let Some(ev) = sub_rx.recv().await {
                let _ = sender_clone.send(ev).await;
            }
        });
        let outcome = self.run_loop(messages, 0, tool_defs, tool_map, selection, &sub_tx, max_iterations, depth, IntentMode::Research, false).await;
        drop(sub_tx);
        let _ = forward_handle.await;
        let (text, iterations, usage, llm_call_count) = match outcome {
            LoopOutcome::Finished { text, iterations, usage, llm_call_count, .. } => (text, iterations, usage, llm_call_count),
            LoopOutcome::EndedWithQuestion { text, iterations, usage, llm_call_count, .. } => (text, iterations, usage, llm_call_count),
            LoopOutcome::Paused { text_buf, iterations, usage, llm_call_count, .. } => (text_buf, iterations, usage, llm_call_count),
        };
        let preview: String = text.chars().take(280).collect();
        let _ = event_sender.send(AgentEvent::ParallelResearchLeadDone {
            index,
            iterations,
            preview,
            duration_ms: lead_start.elapsed().as_millis() as u64,
        }).await;
        (text, iterations, usage, llm_call_count)
    }

    /// Asks the model, without tools, for a final answer from the conversation so far. The
    /// request is appended to `messages`, and usage is added to the turn's running totals.
    async fn synthesize(
        &self,
        messages: &mut Vec<Message>,
        prompt: String,
        selection: Option<&ProviderSelection>,
        last_provider_used: &mut Option<UsedProvider>,
        total_usage: &mut Usage,
        llm_call_count: &mut usize,
    ) -> Synthesis {
        messages.push(Message::user(prompt));
        let _synthesis_permit = self.llm_concurrency.acquire().await;
        match self.llm.chat_routed(selection, messages, &[]).await {
            Ok((response, used, usage)) => {
                *last_provider_used = Some(used);
                *total_usage += usage;
                *llm_call_count += 1;
                let text = match response {
                    LlmResponse::Message { text, .. } => text,
                    LlmResponse::ToolCalls { preamble, .. } => preamble,
                };
                Synthesis { text, error: None }
            }
            Err(e) => {
                tracing::warn!(error = %e, "synthesis request failed");
                Synthesis { text: String::new(), error: Some(e.to_string()) }
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
            "Using ONLY the tool results below, write 1–3 sentences stating the concrete facts \
             you found in the codebase — specific files, functions, config keys, and behavior. \
             Cite each claim with an inline [repo:version:file:line] citation as described in \
             the Citation Rules above. \
             Do not mention asking the user a question, presenting options, or any other future \
             action. Do not use phrases like \"I will\" or \"Let me\". Write as plain findings, \
             not as a plan.\n\n\
             Tool results so far:\n{tool_summary}"
        );
        let mut synth_messages = messages.to_vec();
        synth_messages.push(Message::user(prompt));
        let _synth_permit = self.llm_concurrency.acquire().await;
        match self.llm.chat_routed(selection, &synth_messages, &[]).await {
            Ok((LlmResponse::Message { text, .. }, _, _)) if !text.is_empty() => Some(text),
            Ok((LlmResponse::ToolCalls { preamble, .. }, _, _)) if !preamble.is_empty() => Some(preamble),
            _ => None,
        }
    }

    /// Picks the answer text for a turn ending on `ask_user`, falling back to
    /// synthesis when `candidate` is empty or just narrates the question.
    async fn resolve_ask_user_answer_text(
        &self,
        candidate: String,
        messages: &[Message],
        selection: Option<&ProviderSelection>,
        accumulated_text: &str,
        question: &str,
    ) -> String {
        if !candidate.is_empty() && !is_ask_user_narration(&candidate) {
            return candidate;
        }
        if let Some(synth) = self.synthesize_partial_answer(messages, selection).await {
            return synth;
        }
        if !accumulated_text.is_empty() {
            return accumulated_text.to_string();
        }
        question.to_string()
    }

}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunMetrics {
    pub tool_calls_executed: usize,
    pub tool_errors:         usize,
    pub turns:               usize,
    pub llm_calls:           usize,
    pub duration_ms:         u64,
    pub usage:               Usage,
}

impl RunMetrics {
    pub fn usage_total(&self) -> u64 {
        self.usage.input_tokens + self.usage.output_tokens
    }
}

pub fn run_metrics(
    messages: &[Message],
    turns: usize,
    llm_calls: usize,
    usage: Usage,
    duration_ms: u64,
) -> RunMetrics {
    let (tool_calls_executed, tool_errors) = count_tool_results(messages);
    RunMetrics { tool_calls_executed, tool_errors, turns, llm_calls, duration_ms, usage }
}

pub fn count_tool_results(messages: &[Message]) -> (usize, usize) {
    let mut total = 0;
    let mut errors = 0;
    for msg in messages {
        if let MessageContent::Parts(parts) = &msg.content {
            for part in parts {
                if let ContentPart::ToolResult { is_error, .. } = part {
                    total += 1;
                    if *is_error { errors += 1; }
                }
            }
        }
    }
    (total, errors)
}

fn tally_count(messages: &[Message]) -> usize {
    count_tool_results(messages).0
}

fn tally_errors(messages: &[Message]) -> usize {
    count_tool_results(messages).1
}

fn collect_tool_result_summary(messages: &[Message]) -> String {
    let mut summaries = Vec::new();
    let mut budget = MAX_SUMMARY_TOTAL_CHARS;
    for msg in messages {
        if let MessageContent::Parts(parts) = &msg.content {
            for part in parts {
                if let ContentPart::ToolResult { content, is_error, .. } = part {
                    if budget == 0 { break; }
                    let label = if *is_error { "error" } else { "result" };
                    let entry_budget = budget.min(MAX_SUMMARY_ENTRY_CHARS);
                    let (snippet, truncated) = if content.chars().count() > entry_budget {
                        let head: String = content.chars().take(entry_budget).collect();
                        (format!("{head}… [truncated]"), true)
                    } else {
                        (content.clone(), false)
                    };
                    let line = format!("- [{label}] {snippet}");
                    budget = budget.saturating_sub(line.chars().count());
                    summaries.push(line);
                    if truncated { break; }
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

fn build_synthesis_prompt(prefix: &str, findings: &str, is_uniform: bool) -> String {
    let variation = if is_uniform {
        String::new()
    } else {
        " Check whether your findings differ across the entities, components, \
         drivers, modules, or variants you examined. If they do, state that \
         variation up front in your first sentence instead of leading with a \
         blanket yes or no, and state what you found for each one.\n".to_string()
    };
    format!(
        "{prefix}\n\n\
         {variation}\
         Tool results so far:\n{findings}\n\n\
         If any examined entity lacks a capability the question asks about, \
         state what happens when that capability is required."
    )
}

/// True when `text` narrates asking the user a question rather than stating
/// findings, e.g. "I will now ask the user which approach they prefer."
fn is_ask_user_narration(text: &str) -> bool {
    let lower = text.to_lowercase();
    let announces_asking = lower.contains("ask the user")
        || lower.contains("ask you a")
        || lower.contains("ask a follow")
        || lower.contains("ask for clarification")
        || lower.contains("ask which")
        || lower.contains("ask what")
        || lower.contains("ask whether");
    let future_tense = {
        let t = lower.trim_start();
        t.starts_with("i will") || t.starts_with("i'll") || t.starts_with("let me")
    };
    announces_asking || (future_tense && lower.contains("question"))
}

/// Why a turn ended without an answer from the model.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum IncompleteReason {
    ToolLimit,
    ModelError(String),
    EmptyReply,
}

const INCOMPLETE_ANSWER_PREFIX: &str = "I couldn't finish this answer";

/// The answer shown when the model produced none. It names the actual cause and never
/// dumps raw tool output in place of an answer.
pub(crate) fn incomplete_answer(reason: &IncompleteReason) -> String {
    match reason {
        IncompleteReason::ToolLimit => format!(
            "{INCOMPLETE_ANSWER_PREFIX}: I used the maximum number of tool calls for one turn \
             before I had enough to answer. Ask a narrower question (for example about one \
             component or one file), or ask me to continue."
        ),
        IncompleteReason::ModelError(e) => format!(
            "{INCOMPLETE_ANSWER_PREFIX}: the request to the model failed ({}). Please try again, \
             or pick a different model.",
            short_error(e)
        ),
        IncompleteReason::EmptyReply => format!(
            "{INCOMPLETE_ANSWER_PREFIX}: the model returned an empty reply. Please try again, or \
             pick a different model."
        ),
    }
}

/// True for the placeholder answers produced by [`incomplete_answer`].
pub(crate) fn is_incomplete_answer(text: &str) -> bool {
    text.trim_start().starts_with(INCOMPLETE_ANSWER_PREFIX)
}

const MAX_ERROR_CHARS_IN_ANSWER: usize = 300;

fn short_error(error: &str) -> String {
    let single_line = error.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= MAX_ERROR_CHARS_IN_ANSWER {
        return single_line;
    }
    let head: String = single_line.chars().take(MAX_ERROR_CHARS_IN_ANSWER).collect();
    format!("{head}…")
}

struct Synthesis {
    text: String,
    error: Option<String>,
}

impl Synthesis {
    /// The synthesized text, else the best text streamed earlier in the turn, else an
    /// explanation of why there is no answer.
    fn into_answer(self, accumulated_answer: &str, accumulated_text: &str, reason_if_empty: IncompleteReason) -> String {
        if !self.text.trim().is_empty() {
            return self.text;
        }
        if !accumulated_answer.trim().is_empty() {
            return accumulated_answer.to_string();
        }
        if !accumulated_text.trim().is_empty() {
            return accumulated_text.to_string();
        }
        let reason = match self.error {
            Some(e) => IncompleteReason::ModelError(e),
            None => reason_if_empty,
        };
        incomplete_answer(&reason)
    }
}

/// The text of the most recent user message that the user (or a caller acting as the user)
/// wrote. Tool results travel as user-role messages too, so those are skipped.
fn latest_user_text(messages: &[Message]) -> String {
    messages.iter().rev()
        .filter(|m| matches!(m.role, Role::User))
        .find_map(|m| match &m.content {
            MessageContent::Text(t) => Some(t.clone()),
            MessageContent::Parts(parts) => {
                let text = parts.iter()
                    .filter_map(|p| if let ContentPart::Text { text, .. } = p { Some(text.as_str()) } else { None })
                    .collect::<Vec<_>>()
                    .join(" ");
                if text.is_empty() { None } else { Some(text) }
            }
        })
        .unwrap_or_default()
}

/// Tools that read source or resolved facts, as opposed to locating candidates.
fn is_deep_read_tool(name: &str) -> bool {
    matches!(name,
        "get_symbol_source" | "get_file_symbols" | "find_callers" | "find_callees" | "get_imports"
        | "read_sources" | "get_evidence_pack" | "get_capability_matrix" | "find_subclasses"
    )
}

const SEARCH_NUDGE: &str = "[Note from Harvest, not from the user] You have run several searches \
in a row without reading any results. Read the most relevant results with read_sources (or \
get_evidence_pack for a capability question) instead of searching again, then answer from what \
you already have.";

fn append_to_last_tool_result(messages: &mut [Message], note: &str) {
    for message in messages.iter_mut().rev() {
        if let MessageContent::Parts(parts) = &mut message.content {
            for part in parts.iter_mut().rev() {
                if let ContentPart::ToolResult { content, .. } = part {
                    content.push_str("\n\n");
                    content.push_str(note);
                    return;
                }
            }
        }
    }
}

fn tool_names_by_call_id(messages: &[Message]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for message in messages {
        if let MessageContent::Parts(parts) = &message.content {
            for part in parts {
                if let ContentPart::ToolUse { id, name, .. } = part {
                    names.insert(id.clone(), name.clone());
                }
            }
        }
    }
    names
}

fn relevance_key(tool_use_id: &str, content: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    tool_use_id.hash(&mut hasher);
    content.hash(&mut hasher);
    hasher.finish()
}

fn first_tool_result(message: &Message) -> Option<(String, String)> {
    let MessageContent::Parts(parts) = &message.content else { return None };
    parts.iter().find_map(|p| match p {
        ContentPart::ToolResult { tool_use_id, content, .. } => Some((tool_use_id.clone(), content.clone())),
        _ => None,
    })
}

pub(crate) fn question_fallback() -> String {
    "I've gathered what I can from the codebase. \
     Please answer the question above so I can give you a precise answer."
        .to_string()
}

fn strip_answer_preamble(text: &str) -> String {
    let preamble_patterns = [
        "i am examining", "i am looking at", "i am investigating",
        "i will examine", "i will look at", "i will investigate",
        "i'll examine", "i'll look at", "i'll investigate",
        "let me look at", "let me examine", "let me check",
        "let me investigate", "let me search",
        "based on my research", "based on my analysis",
        "after looking at the code", "after examining",
        "after investigating", "after analyzing",
    ];
    let trimmed = text.trim_start();
    let lower = trimmed.to_lowercase();
    for pattern in &preamble_patterns {
        if lower.starts_with(pattern) {
            if let Some(end) = trimmed.find('.') {
                let after = trimmed[end + 1..].trim_start();
                if !after.is_empty() {
                    return after.to_string();
                }
            }
            if let Some(end) = trimmed.find('\n') {
                let after = trimmed[end + 1..].trim_start();
                if !after.is_empty() {
                    return after.to_string();
                }
            }
        }
    }
    text.to_string()
}

fn derive_phase(tool_calls: &[ToolCall]) -> &'static str {
    let has = |name: &str| tool_calls.iter().any(|c| c.name == name);

    if has("find_callers") || has("find_callees") || has("find_subclasses") || has("run_sql") {
        "Tracing relationships"
    } else if has("get_symbol_source") || has("get_file_symbols") || has("get_imports") || has("compare_symbol_across_versions")
        || has("read_sources") || has("get_evidence_pack") || has("get_capability_matrix") {
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

const MAX_PARALLEL_LEADS: usize = 6;
const MAX_PARALLEL_RESEARCH_DEPTH: usize = 2;
const MAX_TOOL_RESULT_CHARS: usize = 15_000;
const MAX_SUMMARY_ENTRY_CHARS: usize = 1_200;
const MAX_SUMMARY_TOTAL_CHARS: usize = 8_000;

fn cap_tool_result(content: String) -> String {
    if content.chars().count() <= MAX_TOOL_RESULT_CHARS {
        return content;
    }
    let omitted = content.chars().count() - MAX_TOOL_RESULT_CHARS;
    let mut capped: String = content.chars().take(MAX_TOOL_RESULT_CHARS).collect();
    capped.push_str(&format!("\n\n[truncated, {omitted} more characters omitted]"));
    capped
}

fn propose_parallel_research_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "propose_parallel_research".to_string(),
        description: "Call this INSTEAD of investigating directly, as your very first action, \
                      when the user's question splits into 2-6 independent leads that could be \
                      researched in any order without needing each other's findings. Each lead \
                      is investigated separately and the findings are merged into one answer. \
                      Do not call this after you have already started investigating, and do not \
                      call it for a single continuous chain of reasoning."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "leads": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "2–6 independent, self-contained questions to investigate in parallel.",
                    "minItems": 2,
                    "maxItems": 6
                }
            },
            "required": ["leads"]
        }),
    }
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

const MID_TURN_COMPACTION_CHAR_THRESHOLD: usize = 150_000;
const MID_TURN_COMPACTION_KEEP_LAST: usize = 6;

/// Moves a proposed compaction split back so the kept tail never starts with tool results
/// whose tool call would be summarized away. Providers reject a tool result that does not
/// follow its call, so an orphaned result makes every later request in the turn fail.
fn compaction_split_point(messages: &[Message], protected_prefix_len: usize, proposed: usize) -> usize {
    let mut split_at = proposed.min(messages.len());
    while split_at > protected_prefix_len && messages.get(split_at).and_then(first_tool_result).is_some() {
        split_at -= 1;
    }
    split_at
}

fn estimate_messages_chars(messages: &[Message]) -> usize {
    messages.iter().map(message_chars).sum()
}

fn message_chars(message: &Message) -> usize {
    match &message.content {
        MessageContent::Text(text) => text.len(),
        MessageContent::Parts(parts) => parts.iter().map(content_part_chars).sum(),
    }
}

fn content_part_chars(part: &ContentPart) -> usize {
    match part {
        ContentPart::Text { text, .. } => text.len(),
        ContentPart::ToolUse { input, .. } => input.to_string().len(),
        ContentPart::ToolResult { content, .. } => content.len(),
        ContentPart::Image { data, .. } => data.len(),
        ContentPart::Document { data, .. } => data.len(),
    }
}

fn describe_message_for_compaction(message: &Message) -> String {
    let role = match message.role {
        crate::llm::types::Role::System => "System",
        crate::llm::types::Role::User => "User",
        crate::llm::types::Role::Assistant => "Assistant",
        crate::llm::types::Role::Tool => "Tool",
    };
    match &message.content {
        MessageContent::Text(text) => format!("{role}: {text}"),
        MessageContent::Parts(parts) => {
            let rendered: Vec<String> = parts.iter().map(|part| match part {
                ContentPart::Text { text, .. } => text.clone(),
                ContentPart::ToolUse { name, input, .. } => format!("[called {name} with {input}]"),
                ContentPart::ToolResult { content, is_error, .. } => {
                    let label = if *is_error { "tool error" } else { "tool result" };
                    format!("[{label}: {content}]")
                }
                ContentPart::Image { .. } => "[image attachment]".to_string(),
                ContentPart::Document { .. } => "[document attachment]".to_string(),
            }).collect();
            format!("{role}: {}", rendered.join(" "))
        }
    }
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

fn parse_line_range(raw: &str) -> (u32, Option<u32>) {
    let first = raw.split(',').next().unwrap_or("");
    let mut parts = first.splitn(2, |c: char| c == '-' || c == '\u{2013}');
    let start = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let end = parts.next().and_then(|s| s.parse().ok());
    (start, end)
}

fn split_citation_body(body: &str) -> Vec<String> {
    let mut groups: Vec<String> = Vec::new();
    for piece in body.split(',') {
        let trimmed = piece.trim();
        if trimmed.matches(':').count() >= 2 || groups.is_empty() {
            groups.push(trimmed.to_string());
        } else if let Some(last) = groups.last_mut() {
            last.push(',');
            last.push_str(trimmed);
        }
    }
    groups
}

fn parse_citations(text: &str) -> Vec<Source> {
    // The line number (and range) is optional: a model sometimes cites a whole
    // file rather than a specific location (e.g. [repo:v1.0:src/lib.rs] to
    // support a claim about the file's overall purpose). Line 0 doubles as the
    // "no specific line" sentinel, matching how an explicit ":0" already parses.
    let bracket_re = Regex::new(r"\[([^\[\]]+)\]").unwrap();
    let citation_re = Regex::new(r"^([^:\s]+):([^:\s]+):([^:\s]+)(?::(\d+(?:[–-]\d+)?(?:,\d+(?:[–-]\d+)?)*))?$").unwrap();
    let mut seen = HashSet::new();
    let mut sources = Vec::new();

    for bracket in bracket_re.captures_iter(text) {
        for group in split_citation_body(&bracket[1]) {
            let Some(cap) = citation_re.captures(&group) else { continue };
            if seen.insert(group.clone()) {
                let (line, end_line) = cap.get(4)
                    .map(|m| parse_line_range(m.as_str()))
                    .unwrap_or((0, None));
                sources.push(Source {
                    repo: cap[1].to_string(),
                    version: cap[2].to_string(),
                    file: cap[3].to_string(),
                    line,
                    end_line,
                });
            }
        }
    }
    sources
}

#[cfg(test)]
mod tests {
    #[test]
    fn query_response_reports_the_exact_tool_call_count() {
        let metrics = RunMetrics {
            tool_calls_executed: 7,
            tool_errors: 2,
            turns: 3,
            llm_calls: 4,
            duration_ms: 11,
            usage: Usage::default(),
        };
        let r = QueryResponse::from_metrics(
            "answer".into(),
            Vec::new(),
            Some(UsedProvider { provider_id: "gemini".into(), kind: "gemini".into(), model: "m".into() }),
            11,
            4,
            &metrics,
        );
        assert_eq!(r.tool_calls_made, 7, "the response must carry the exact tool count, not the turn count");
        assert_eq!(r.tool_errors, 2);
        assert_eq!(r.llm_call_count, 4);
        assert_eq!(r.turns, 3);
    }

    #[test]
    fn query_response_exposes_zero_counts_for_a_clean_run() {
        let metrics = RunMetrics {
            tool_calls_executed: 0,
            tool_errors: 0,
            turns: 1,
            llm_calls: 1,
            duration_ms: 5,
            usage: Usage::default(),
        };
        let r = QueryResponse::from_metrics("answer".into(), Vec::new(), None, 5, 1, &metrics);
        assert_eq!(r.tool_calls_made, 0);
        assert_eq!(r.tool_errors, 0);
    }

    fn result_message(is_error: bool) -> Message {
        Message {
            role: Role::User,
            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                tool_use_id: "id".into(),
                content: "ok".into(),
                is_error,
            }]),
        }
    }

    #[test]
    fn exact_tool_counts_differ_from_the_turn_count() {
        let messages = vec![result_message(false), result_message(false), result_message(true)];
        let (total, errors) = count_tool_results(&messages);
        assert_eq!((total, errors), (3, 1));
        assert_ne!(total, 1, "the tool count is not the turn count");
    }

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

    struct RecordingLlm {
        responses: Mutex<VecDeque<LlmResponse>>,
        requests: Mutex<Vec<String>>,
    }

    impl RecordingLlm {
        fn new(responses: Vec<LlmResponse>) -> Arc<Self> {
            Arc::new(Self { responses: Mutex::new(responses.into()), requests: Mutex::new(Vec::new()) })
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LlmProvider for RecordingLlm {
        fn id(&self) -> &str { "recording-llm" }
        fn kind(&self) -> &str { "mock" }
        fn default_model(&self) -> &str { "mock-model" }

        async fn list_models(&self) -> Result<Vec<crate::llm::types::ModelInfo>> { Ok(vec![]) }

        async fn chat_with(&self, _model: Option<&str>, messages: &[Message], _tools: &[ToolDefinition]) -> Result<LlmResponse> {
            let last_user = messages.iter().rev()
                .find(|m| matches!(m.role, crate::llm::types::Role::User))
                .map(|m| match &m.content {
                    MessageContent::Text(t) => t.clone(),
                    MessageContent::Parts(parts) => parts.iter()
                        .filter_map(|p| if let ContentPart::Text { text, .. } = p { Some(text.clone()) } else { None })
                        .collect::<Vec<_>>()
                        .join(" "),
                })
                .unwrap_or_default();
            self.requests.lock().unwrap().push(last_user);
            self.responses.lock().unwrap().pop_front()
                .ok_or_else(|| anyhow::anyhow!("RecordingLlm: no more responses"))
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
            usage: Usage::default(),
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
            usage: Usage::default(),
        }
    }

    fn propose_call(leads: Vec<&str>) -> LlmResponse {
        LlmResponse::ToolCalls {
            calls: vec![ToolCall {
                id: "tc_propose".into(),
                name: "propose_parallel_research".into(),
                input: serde_json::json!({ "leads": leads }),
                thought_signature: None,
            }],
            preamble: String::new(),
            usage: Usage::default(),
        }
    }

    fn text(s: &str) -> LlmResponse {
        LlmResponse::Message { text: s.into(), usage: Usage::default() }
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
        let selection = ProviderSelection { provider_id: "mock-llm".into(), model: Some("custom-model".into()), cache_breakpoint_index: None };
        let resp = agent.query("hi", &[], &[], Some(&selection)).await.unwrap();
        let used = resp.provider_used.expect("expected provider_used to be set");
        assert_eq!(used.provider_id, "mock-llm");
        assert_eq!(used.model, "custom-model");
    }

    #[tokio::test]
    async fn one_tool_call_then_text_counts_one_iteration() {
        let llm = MockLlm::new(vec![
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
            tool_call("nonexistent_tool"),
            text("handled gracefully"),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("some_tool", "ok")], 5);
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "handled gracefully");
    }

    #[tokio::test]
    async fn max_tokens_auto_continues_then_end_turn() {
        let llm = MockStreamingLlm::new(vec![
            vec![
                StreamEvent::TextDelta { text: "Hello ".into() },
                StreamEvent::TextDelta { text: "world".into() },
                StreamEvent::Done { stop_reason: "max_tokens".into(), usage: Usage::default() },
            ],
            vec![
                StreamEvent::TextDelta { text: " continued".into() },
                StreamEvent::Done { stop_reason: "end_turn".into(), usage: Usage::default() },
            ],
        ]);
        let agent = Arc::new(agent_with(llm, vec![], 5));
        let events = collect_agent_events(agent, "hi").await;

        let done = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, tool_calls_made, .. } => Some((answer.clone(), *tool_calls_made)),
            _ => None,
        }).expect("expected Done event");
        assert_eq!(done.0, "Hello world continued");
        assert_eq!(done.1, 0, "no tools were executed, so the exact count is zero rather than the turn count");
    }

    #[tokio::test]
    async fn max_tokens_without_remaining_iterations_returns_partial() {
        let llm = MockStreamingLlm::new(vec![
            vec![
                StreamEvent::TextDelta { text: "partial text".into() },
                StreamEvent::Done { stop_reason: "max_tokens".into(), usage: Usage::default() },
            ],
        ]);
        let agent = Arc::new(agent_with(llm, vec![], 1));
        let events = collect_agent_events(agent, "hi").await;

        let done = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        }).expect("expected Done event");
        assert_eq!(done, "partial text");
    }

    #[tokio::test]
    async fn query_does_not_deadlock_with_many_events() {
        let mut round1 = Vec::new();
        for i in 0..100 {
            round1.push(StreamEvent::ThinkingDelta { text: format!("thinking chunk {i} ") });
        }
        round1.push(StreamEvent::TextDelta { text: "final answer".into() });
        round1.push(StreamEvent::Done { stop_reason: "end_turn".into(), usage: Usage::default() });
        let llm = MockStreamingLlm::new(vec![round1]);
        let agent = agent_with(llm, vec![], 5);
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "final answer");
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
    async fn normal_completion_with_empty_text_explains_instead_of_dumping_results() {
        let llm = MockLlm::new(vec![
            tool_call("my_tool"),
            text(""),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "found something useful")], 5);
        let resp = agent.query("hi", &[], &[], None).await.unwrap();
        assert!(resp.tool_calls_made < 5, "should finish before hitting max_iterations, got {}", resp.tool_calls_made);
        assert!(is_incomplete_answer(&resp.answer), "got: {}", resp.answer);
        assert!(!resp.answer.contains("found something useful"), "raw tool output must not be the answer, got: {}", resp.answer);
        assert!(!resp.answer.contains("maximum number of tool calls"), "the tool limit was not hit, got: {}", resp.answer);
    }

    #[tokio::test]
    async fn max_iterations_preserves_accumulated_text() {
        let llm = MockLlm::new(vec![
            LlmResponse::ToolCalls {
                calls: vec![ToolCall { id: "tc_1".into(), name: "my_tool".into(), input: serde_json::json!({}), thought_signature: None }],
                preamble: "I found something".into(),
            usage: Usage::default(),
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
        let llm = MockLlm::new(vec![]);
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
    fn derive_phase_tracing_for_run_sql() {
        let calls = vec![tool_call_obj("run_sql", "SELECT 1")];
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
    async fn done_event_marks_hit_max_iterations_when_synthesis_fallback_taken() {
        let llm = MockLlm::new(vec![
            tool_call("search_symbols"),
        ]);
        let agent = Arc::new(agent_with(
            llm,
            vec![MockTool::new("search_symbols", "ok")],
            1,
        ));
        let events = collect_agent_events(agent, "how does X work?").await;

        let hit_max_iterations = events.iter().find_map(|e| match e {
            AgentEvent::Done { hit_max_iterations, .. } => Some(*hit_max_iterations),
            _ => None,
        });
        assert_eq!(hit_max_iterations, Some(true));
    }

    #[tokio::test]
    async fn done_event_does_not_mark_hit_max_iterations_on_normal_completion() {
        let agent = Arc::new(agent_with(MockLlm::new(vec![text("hello world")]), vec![], 5));
        let events = collect_agent_events(agent, "hi").await;

        let hit_max_iterations = events.iter().find_map(|e| match e {
            AgentEvent::Done { hit_max_iterations, .. } => Some(*hit_max_iterations),
            _ => None,
        });
        assert_eq!(hit_max_iterations, Some(false));
    }

    #[tokio::test]
    async fn classify_intent_defaults_to_research_for_ambiguous_followup_without_an_llm_call() {
        let llm = MockLlm::new(vec![]);
        let agent = agent_with(llm, vec![MockTool::new("search_symbols", "ok")], 5);
        let history = vec![
            HistoryMessage { role: "user".into(), text: "how does retry work?".into(), attachments: None },
            HistoryMessage { role: "assistant".into(), text: "The retry logic lives in retry.rs…".into(), attachments: None },
        ];
        let mode = agent.classify_intent("what does that mean?", &history, None).await;
        assert_eq!(mode, IntentMode::Research);
    }

    #[test]
    fn build_tool_defs_excludes_propose_parallel_research_by_default() {
        let llm = MockLlm::new(vec![]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5);
        let defs = agent.build_tool_defs();
        assert!(
            !defs.iter().any(|d| d.name == "propose_parallel_research"),
            "a procedural task agent (e.g. deployment design generation) must not see this tool \
             unless explicitly opted in — its own prompt has no idea the tool exists, so a model \
             offered it may 'split' a fixed write-then-save procedure into independent leads and \
             silently drop the save step"
        );
    }

    #[test]
    fn build_tool_defs_includes_propose_parallel_research_when_enabled() {
        let llm = MockLlm::new(vec![]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "ok")], 5)
            .with_parallel_research(true);
        let defs = agent.build_tool_defs();
        assert!(defs.iter().any(|d| d.name == "propose_parallel_research"));
    }

    #[tokio::test]
    async fn research_mode_with_two_subtasks_runs_parallel_and_merges() {
        let llm = MockLlm::new(vec![
            propose_call(vec!["how auth retries", "how billing retries"]),
            text("finding from a lead"),
            text("finding from a lead"),
            text("merged answer"),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("search_symbols", "ok")], 5));
        let events = collect_agent_events(agent, "compare how auth and billing handle retries").await;

        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::ParallelResearchStarted { leads } if leads.len() == 2)),
            "expected a ParallelResearchStarted event naming both leads"
        );
        assert_eq!(
            events.iter().filter(|e| matches!(e, AgentEvent::ParallelResearchLeadDone { .. })).count(),
            2,
            "expected one ParallelResearchLeadDone per lead"
        );
        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::ParallelResearchMergeStarted { .. })),
            "expected a ParallelResearchMergeStarted event once leads finished"
        );

        // The durable events must arrive in fan-out order, and before the merge's own
        // visible activity — this is what lets the UI render lanes that fill in as
        // leads finish, then converge into the merge step.
        let started_pos = events.iter().position(|e| matches!(e, AgentEvent::ParallelResearchStarted { .. })).unwrap();
        let last_lead_done_pos = events.iter().rposition(|e| matches!(e, AgentEvent::ParallelResearchLeadDone { .. })).unwrap();
        let merge_started_pos = events.iter().position(|e| matches!(e, AgentEvent::ParallelResearchMergeStarted { .. })).unwrap();
        assert!(started_pos < last_lead_done_pos, "leads must complete after the fan-out starts");
        assert!(last_lead_done_pos < merge_started_pos, "merge must start only after every lead is done");

        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        });
        assert_eq!(answer.as_deref(), Some("merged answer"));
    }

    #[tokio::test]
    async fn parallel_research_merge_prompt_caps_gap_filling_calls() {
        let llm = CapturingLlm::new(vec![
            propose_call(vec!["how auth retries", "how billing retries"]),
            text("finding from a lead"),
            text("finding from a lead"),
            text("merged answer"),
        ]);
        let agent = Arc::new(Agent::new(
            Arc::clone(&llm) as Arc<dyn LlmProvider>,
            vec![MockTool::new("search_symbols", "ok")],
            5,
        ));
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        agent.query_streaming("compare how auth and billing handle retries", &[], &[], None, tx).await;
        while rx.try_recv().is_ok() {}

        let captured = llm.captured_last_user_texts.lock().unwrap();
        assert!(
            captured.iter().any(|t| t.contains("at most 2 such gap-filling calls")),
            "expected the merge prompt to cap gap-filling calls to a concrete number, got: {captured:?}"
        );
    }

    #[tokio::test]
    async fn parallel_research_prompts_ask_leads_and_merge_to_check_coverage_symmetry() {
        let llm = CapturingLlm::new(vec![
            propose_call(vec!["how auth retries", "how billing retries"]),
            text("finding from a lead"),
            text("finding from a lead"),
            text("merged answer"),
        ]);
        let agent = Arc::new(Agent::new(
            Arc::clone(&llm) as Arc<dyn LlmProvider>,
            vec![MockTool::new("search_symbols", "ok")],
            5,
        ));
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        agent.query_streaming("compare how auth and billing handle retries", &[], &[], None, tx).await;
        while rx.try_recv().is_ok() {}

        let captured = llm.captured_last_user_texts.lock().unwrap();
        assert!(
            captured.iter().any(|t| t.contains("cover the same kind of ground")),
            "expected the per-lead prompt to ask for comparable coverage, got: {captured:?}"
        );
        assert!(
            captured.iter().any(|t| t.contains("check whether the leads cover comparable ground")),
            "expected the merge prompt to ask for a symmetry check, got: {captured:?}"
        );
    }

    #[tokio::test]
    async fn propose_with_fewer_than_two_leads_falls_through_to_normal_handling() {
        let llm = MockLlm::new(vec![
            propose_call(vec!["only one lead"]),
            text("direct answer"),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("search_symbols", "ok")], 5));
        let events = collect_agent_events(agent, "how does X work?").await;

        assert!(!events.iter().any(|e| matches!(e, AgentEvent::ParallelResearchStarted { .. })));
        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        });
        assert_eq!(answer.as_deref(), Some("direct answer"));
    }

    #[tokio::test]
    async fn propose_after_a_quick_discovery_step_still_triggers_parallel_research() {
        let llm = MockLlm::new(vec![
            tool_call("list_repositories"),
            propose_call(vec!["how auth retries", "how billing retries"]),
            text("finding from a lead"),
            text("finding from a lead"),
            text("merged answer"),
        ]);
        let agent = Arc::new(agent_with(
            llm,
            vec![MockTool::new("list_repositories", "landscape-server"), MockTool::new("search_symbols", "ok")],
            10,
        ));
        let events = collect_agent_events(agent, "compare how auth and billing handle retries").await;

        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::ParallelResearchStarted { leads } if leads.len() == 2)),
            "a discovery step first should not block a subsequent propose call"
        );
        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        });
        assert_eq!(answer.as_deref(), Some("merged answer"));
    }

    #[tokio::test]
    async fn propose_after_several_discovery_steps_still_triggers_parallel_research() {
        // The model isn't required to decide upfront — recognizing a split
        // partway through investigating should still fork the remaining work.
        let llm = MockLlm::new(vec![
            tool_call("my_tool"),
            tool_call("my_tool"),
            tool_call("my_tool"),
            propose_call(vec!["lead one", "lead two"]),
            text("finding from a lead"),
            text("finding from a lead"),
            text("merged answer"),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("my_tool", "ok")], 10));
        let events = collect_agent_events(agent, "hi").await;

        assert!(events.iter().any(|e| matches!(e, AgentEvent::ParallelResearchStarted { leads } if leads.len() == 2)),
            "a propose call arriving after some discovery steps should still fork");
        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        });
        assert_eq!(answer.as_deref(), Some("merged answer"));
    }

    #[tokio::test]
    async fn parallel_research_never_offers_confirmable_tools_to_leads() {
        let llm = MockLlm::new(vec![
            propose_call(vec!["lead one", "lead two"]),
            text("finding from a lead"),
            text("finding from a lead"),
            text("merged answer"),
        ]);
        let agent = Arc::new(agent_with(
            llm,
            vec![MockTool::new_confirmable("dangerous_tool", "should never run silently")],
            5,
        ));
        let events = collect_agent_events(agent, "compare two independent leads").await;

        assert!(!events.iter().any(|e| matches!(e, AgentEvent::ConfirmAction { .. })));
        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        });
        assert_eq!(answer.as_deref(), Some("merged answer"));
    }

    #[tokio::test]
    async fn a_lead_can_itself_fan_out_one_nested_level() {
        let llm = MockLlm::new(vec![
            propose_call(vec!["lead A", "lead B"]),
            propose_call(vec!["lead A1", "lead A2"]),
            text("finding A1"),
            text("finding A2"),
            text("merged A"),
            text("finding B"),
            text("final merged answer"),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("search_symbols", "ok")], 20));
        let events = collect_agent_events(agent, "compare two independent leads, one of which is itself broad").await;

        let started_leads_counts: Vec<usize> = events.iter().filter_map(|e| match e {
            AgentEvent::ParallelResearchStarted { leads } => Some(leads.len()),
            _ => None,
        }).collect();
        assert!(started_leads_counts.iter().any(|&n| n == 2),
            "expected at least the top-level fan-out event, got: {started_leads_counts:?}");

        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        });
        assert_eq!(answer.as_deref(), Some("final merged answer"));
    }

    #[tokio::test]
    async fn query_streaming_emits_intent_event_first() {
        let llm = MockLlm::new(vec![
            text("answer"),
        ]);
        let agent = Arc::new(agent_with(llm, vec![MockTool::new("search_symbols", "ok")], 5));
        let events = collect_agent_events(agent, "how does X work?").await;
        assert!(matches!(events[0], AgentEvent::Intent { mode: IntentMode::Research }));
    }

    #[tokio::test]
    async fn direct_text_answer_makes_no_tool_calls_even_with_tools_available() {
        let llm = MockLlm::new(vec![
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
        assert!(!has_tool, "the model chose not to call any tool, so none should fire");
        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        });
        assert_eq!(answer.as_deref(), Some("direct answer"));
    }

    #[tokio::test]
    async fn multiple_tool_calls_in_one_turn_all_executed() {
        let llm = MockLlm::new(vec![
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
        assert_eq!(resp.tool_calls_made, 2, "both tools were executed in a single turn");
        assert_eq!(resp.tool_errors, 0);
        assert_eq!(resp.turns, 1, "the turn count is tracked separately from the tool count");
    }

    #[tokio::test]
    async fn multi_turn_query_reports_provider_used_from_final_call() {
        let llm = MockLlm::with_id("mock-llm", vec![
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
    fn cap_tool_result_leaves_short_content_untouched() {
        let content = "a short tool result".to_string();
        assert_eq!(cap_tool_result(content.clone()), content);
    }

    #[test]
    fn cap_tool_result_truncates_long_content_with_marker() {
        let content = "x".repeat(MAX_TOOL_RESULT_CHARS + 500);
        let capped = cap_tool_result(content);
        assert!(capped.len() < MAX_TOOL_RESULT_CHARS + 500);
        assert!(capped.contains("truncated"));
        assert!(capped.contains("500 more characters omitted"));
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
    fn two_citations_combined_in_one_bracket_parsed_separately() {
        let sources = parse_citations("[repo:v1:a.rs:1, repo:v1:b.rs:2]");
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].file, "a.rs");
        assert_eq!(sources[0].line, 1);
        assert_eq!(sources[1].file, "b.rs");
        assert_eq!(sources[1].line, 2);
    }

    #[test]
    fn combined_bracket_with_multi_range_first_citation_still_splits() {
        let sources = parse_citations("[repo:v1:a.rs:32-48,77-86, repo:v1:b.rs:5]");
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].file, "a.rs");
        assert_eq!(sources[0].line, 32);
        assert_eq!(sources[0].end_line, Some(48));
        assert_eq!(sources[1].file, "b.rs");
        assert_eq!(sources[1].line, 5);
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
        let sources = parse_citations("bad [only:two] here");
        assert!(sources.is_empty());
    }

    #[test]
    fn citation_without_line_number_parsed_as_whole_file() {
        let sources = parse_citations("see [myrepo:v1.0:src/lib.rs]");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].repo, "myrepo");
        assert_eq!(sources[0].version, "v1.0");
        assert_eq!(sources[0].file, "src/lib.rs");
        assert_eq!(sources[0].line, 0);
        assert_eq!(sources[0].end_line, None);
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
        assert_eq!(sources[0].end_line, Some(335));
    }

    #[test]
    fn citation_single_line_has_no_end_line() {
        let sources = parse_citations("[r:v1:f.rs:42]");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].end_line, None);
    }

    #[test]
    fn citation_with_multiple_ranges_uses_first_range() {
        let sources = parse_citations("[r:v1:f.rs:32-48,77-86]");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].line, 32);
        assert_eq!(sources[0].end_line, Some(48));
    }

    #[test]
    fn citation_with_en_dash_range_parsed() {
        let sources = parse_citations("[r:v1:f.rs:32\u{2013}48]");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].line, 32);
        assert_eq!(sources[0].end_line, Some(48));
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

    #[test]
    fn tool_result_summary_preserves_capability_declaration() {
        let body = "x".repeat(600);
        let content = format!("{body}\n    SUPPORTS_ACTIVE_ACTIVE = True\n");
        let messages = vec![tool_result_message("tc_1", content)];
        let summary = collect_tool_result_summary(&messages);
        assert!(
            summary.contains("SUPPORTS_ACTIVE_ACTIVE = True"),
            "capability declaration was truncated away: {summary}"
        );
    }

    #[test]
    fn tool_result_summary_caps_each_entry_at_budget() {
        let content = "y".repeat(MAX_SUMMARY_ENTRY_CHARS * 3);
        let messages = vec![tool_result_message("tc_1", content)];
        let summary = collect_tool_result_summary(&messages);
        assert!(summary.chars().count() < MAX_SUMMARY_ENTRY_CHARS * 2);
        assert!(summary.contains("truncated"));
    }

    #[test]
    fn tool_result_summary_bounds_total_length() {
        let messages: Vec<Message> = (0..40)
            .map(|i| tool_result_message(&format!("tc_{i}"), "z".repeat(4000)))
            .collect();
        let summary = collect_tool_result_summary(&messages);
        assert!(
            summary.chars().count() <= MAX_SUMMARY_TOTAL_CHARS + 200,
            "summary was {} chars, over budget",
            summary.chars().count()
        );
    }

    #[test]
    fn tool_result_summary_marks_errors() {
        let messages = vec![Message {
            role: crate::llm::types::Role::User,
            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                tool_use_id: "tc_1".into(),
                content: "query failed".into(),
                is_error: true,
            }]),
        }];
        assert!(collect_tool_result_summary(&messages).contains("[error]"));
    }

    #[test]
    fn tool_result_summary_reports_when_nothing_collected() {
        let summary = collect_tool_result_summary(&[Message::user("hi")]);
        assert!(summary.contains("No tool results"));
    }

    fn tool_result_message(id: &str, content: String) -> Message {
        Message {
            role: crate::llm::types::Role::User,
            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                tool_use_id: id.into(),
                content,
                is_error: false,
            }]),
        }
    }

    #[test]
    fn run_metrics_count_tool_results_and_errors() {
        let messages = vec![
            tool_result_message("tc_1", "first".into()),
            tool_result_message("tc_2", "second".into()),
            Message {
                role: crate::llm::types::Role::User,
                content: MessageContent::Parts(vec![ContentPart::ToolResult {
                    tool_use_id: "tc_3".into(),
                    content: "boom".into(),
                    is_error: true,
                }]),
            },
        ];
        let metrics = run_metrics(&messages, 2, 4, Usage::default(), 1000);
        assert_eq!(metrics.tool_calls_executed, 3);
        assert_eq!(metrics.tool_errors, 1);
        assert_eq!(metrics.turns, 2);
        assert_eq!(metrics.llm_calls, 4);
        assert_eq!(metrics.duration_ms, 1000);
    }

    #[test]
    fn run_metrics_report_zero_for_conversation_without_tools() {
        let metrics = run_metrics(&[Message::user("hello")], 1, 0, Usage::default(), 5);
        assert_eq!(metrics.tool_calls_executed, 0);
        assert_eq!(metrics.tool_errors, 0);
    }

    #[test]
    fn run_metrics_accumulate_usage_across_turns() {
        let mut usage = Usage::default();
        usage.input_tokens = 1200;
        usage.output_tokens = 340;
        usage.cache_read_tokens = 900;
        let metrics = run_metrics(&[], 1, 1, usage, 10);
        assert_eq!(metrics.usage.input_tokens, 1200);
        assert_eq!(metrics.usage.output_tokens, 340);
        assert_eq!(metrics.usage.cache_read_tokens, 900);
        assert_eq!(metrics.usage_total(), 1540);
    }

    #[tokio::test]
    async fn compact_messages_mid_turn_returns_unchanged_under_threshold() {
        let agent = agent_with(MockLlm::new(vec![]), vec![], 5);
        let messages = vec![
            Message::system("system prompt"),
            Message::user("question"),
            tool_result_message("tc_1", "short result".into()),
        ];
        let result = agent.compact_messages_mid_turn(messages.clone(), 2).await;
        assert_eq!(result.len(), messages.len());
    }

    #[tokio::test]
    async fn compact_messages_mid_turn_compacts_when_over_threshold() {
        let agent = agent_with(MockLlm::new(vec![text("condensed summary of the trace")]), vec![], 5);
        let protected = vec![Message::system("system prompt"), Message::user("question")];
        let big_result = "x".repeat(20_000);
        let mut messages = protected.clone();
        for i in 0..10 {
            let id = format!("tc_{i}");
            messages.push(tool_use_message(&[id.as_str()]));
            messages.push(tool_result_message(&id, big_result.clone()));
        }
        let result = agent.compact_messages_mid_turn(messages.clone(), protected.len()).await;

        assert_eq!(result.len(), protected.len() + 1 + MID_TURN_COMPACTION_KEEP_LAST);
        match &result[protected.len()].content {
            MessageContent::Text(text) => assert!(text.contains("condensed summary of the trace")),
            other => panic!("expected summary text message, got {other:?}"),
        }
        let kept_start = messages.len() - MID_TURN_COMPACTION_KEEP_LAST;
        for (kept, original) in result[protected.len() + 1..].iter().zip(&messages[kept_start..]) {
            match (&kept.content, &original.content) {
                (MessageContent::Parts(a), MessageContent::Parts(b)) => {
                    let id_of = |part: &ContentPart| match part {
                        ContentPart::ToolResult { tool_use_id, .. } => tool_use_id.clone(),
                        ContentPart::ToolUse { id, .. } => id.clone(),
                        other => panic!("expected a tool call or result, got {other:?}"),
                    };
                    assert_eq!(id_of(&a[0]), id_of(&b[0]));
                }
                other => panic!("expected matching tool parts, got {other:?}"),
            }
        }
        assert!(first_tool_result(&result[protected.len() + 1]).is_none(), "the kept tail must start with a tool call");
    }

    #[tokio::test]
    async fn compact_messages_mid_turn_falls_back_on_llm_failure() {
        let agent = agent_with(MockLlm::new(vec![]), vec![], 5);
        let protected = vec![Message::system("system prompt"), Message::user("question")];
        let big_result = "x".repeat(20_000);
        let mut messages = protected.clone();
        for i in 0..10 {
            messages.push(tool_result_message(&format!("tc_{i}"), big_result.clone()));
        }
        let result = agent.compact_messages_mid_turn(messages.clone(), protected.len()).await;
        assert_eq!(result.len(), messages.len());
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
        captured_last_user_texts: Mutex<Vec<String>>,
    }

    impl CapturingLlm {
        fn new(responses: Vec<LlmResponse>) -> Arc<Self> {
            Arc::new(Self {
                response: Mutex::new(responses.into()),
                captured_system: Mutex::new(None),
                captured_last_user_texts: Mutex::new(Vec::new()),
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
            if let Some(MessageContent::Text(t)) = messages.last().map(|m| &m.content) {
                self.captured_last_user_texts.lock().unwrap().push(t.clone());
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

        assert_eq!(llm.captured_system.lock().unwrap().as_deref(), Some(prompt::system_prompt(false).as_str()));
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
            LlmResponse::ToolCalls {
                calls: vec![ToolCall { id: "t".into(), name: "my_tool".into(), input: serde_json::json!({}), thought_signature: None }],
                preamble: "Let me check that".into(),
            usage: Usage::default(),
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
                StreamEvent::Done { stop_reason: "tool_use".into(), usage: Usage::default() },
            ],
            vec![
                StreamEvent::TextDelta { text: "done".into() },
                StreamEvent::Done { stop_reason: "end_turn".into(), usage: Usage::default() },
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
            usage: Usage::default(),
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
            paused.elapsed_ms,
        ).await;
        assert!(outcome.is_none(), "expected the turn to finish, not pause again");

        let mut events = Vec::new();
        while let Ok(e) = rx2.try_recv() { events.push(e); }
        let answer = events.iter().find_map(|e| match e {
            AgentEvent::Done { answer, .. } => Some(answer.clone()),
            _ => None,
        });
        assert_eq!(answer.as_deref(), Some("Done, agent deleted."));

        let duration_ms = events.iter().find_map(|e| match e {
            AgentEvent::Done { duration_ms, .. } => Some(*duration_ms),
            _ => None,
        });
        assert!(duration_ms.unwrap() >= paused.elapsed_ms, "resumed duration should include time spent before the pause");
    }

    #[tokio::test]
    async fn resume_after_confirm_can_pause_again_on_a_second_confirmable_call() {
        let llm = MockLlm::new(vec![
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
            first.elapsed_ms,
        ).await.expect("expected the resumed turn to pause again");

        assert_eq!(second.pending, vec![PendingConfirmCall { id: "tc_1:0".into(), tool_use_id: "tc_1".into() }]);
    }

    #[tokio::test]
    async fn non_confirmable_tool_calls_execute_normally_alongside_confirmable_ones_absent() {
        let llm = MockLlm::new(vec![
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
            LlmResponse::ToolCalls {
                calls: vec![ToolCall {
                    id: "ask_1".into(),
                    name: "ask_user".into(),
                    input: serde_json::json!({"question": "Which aspect?", "choices": ["A", "B"]}),
                    thought_signature: None,
                }],
                preamble: String::new(),
            usage: Usage::default(),
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
            // Iteration 1: tool call WITH preamble text
            LlmResponse::ToolCalls {
                calls: vec![ToolCall {
                    id: "tc_1".into(),
                    name: "my_tool".into(),
                    input: serde_json::json!({}),
                    thought_signature: None,
                }],
                preamble: "I found some relevant code.".into(),
            usage: Usage::default(),
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
            usage: Usage::default(),
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
            // Iteration 1: tool call with no text preamble
            LlmResponse::ToolCalls {
                calls: vec![ToolCall {
                    id: "tc_1".into(),
                    name: "my_tool".into(),
                    input: serde_json::json!({}),
                    thought_signature: None,
                }],
                preamble: String::new(),
            usage: Usage::default(),
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
            usage: Usage::default(),
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
    fn incomplete_answer_names_the_actual_cause() {
        let limit = incomplete_answer(&IncompleteReason::ToolLimit);
        assert!(limit.contains("maximum number of tool calls"), "{limit}");
        let failed = incomplete_answer(&IncompleteReason::ModelError("HTTP 400: bad tool order".into()));
        assert!(failed.contains("HTTP 400: bad tool order"), "{failed}");
        assert!(!failed.contains("tool calls"), "a model failure must not be blamed on the tool limit: {failed}");
        let empty = incomplete_answer(&IncompleteReason::EmptyReply);
        assert!(empty.contains("empty reply"), "{empty}");
        for answer in [limit, failed, empty] {
            assert!(is_incomplete_answer(&answer), "{answer}");
        }
        assert!(!is_incomplete_answer("The handler lives in foo.rs."));
    }

    #[test]
    fn incomplete_answer_shortens_long_errors_to_one_line() {
        let long = format!("line one\nline two {}", "x".repeat(1000));
        let answer = incomplete_answer(&IncompleteReason::ModelError(long));
        assert!(!answer.contains('\n'), "{answer}");
        assert!(answer.chars().count() < 600, "{answer}");
    }

    #[test]
    fn latest_user_text_skips_tool_results() {
        let messages = vec![
            Message::system("system"),
            Message::user("Which drivers support HA?"),
            Message { role: Role::Assistant, content: MessageContent::Parts(vec![ContentPart::ToolUse {
                id: "tc_1".into(), name: "search_symbols".into(), input: serde_json::json!({}), thought_signature: None,
            }]) },
            result_message(false),
        ];
        assert_eq!(latest_user_text(&messages), "Which drivers support HA?");
    }

    #[test]
    fn append_to_last_tool_result_keeps_the_note_inside_the_result() {
        let mut messages = vec![Message::user("q"), result_message(false), result_message(false)];
        append_to_last_tool_result(&mut messages, "NOTE");
        let contents: Vec<String> = messages.iter().filter_map(first_tool_result).map(|(_, c)| c).collect();
        assert_eq!(contents, vec!["ok".to_string(), "ok\n\nNOTE".to_string()]);
        assert_eq!(messages.len(), 3, "the note must not add a message");
    }

    #[test]
    fn batch_reads_count_as_reading_source() {
        for name in ["read_sources", "get_evidence_pack", "get_capability_matrix", "get_symbol_source"] {
            assert!(is_deep_read_tool(name), "{name}");
        }
        assert!(!is_deep_read_tool("search_symbols"));
    }

    fn tool_use_message(ids: &[&str]) -> Message {
        Message { role: Role::Assistant, content: MessageContent::Parts(ids.iter().map(|id| ContentPart::ToolUse {
            id: (*id).into(), name: "read_sources".into(), input: serde_json::json!({}), thought_signature: None,
        }).collect()) }
    }

    #[test]
    fn compaction_split_never_orphans_tool_results() {
        // prefix: system + user; then a call with four results, then a call with one result.
        let mut messages = vec![Message::system("s"), Message::user("q"), tool_use_message(&["a", "b", "c", "d"])];
        messages.extend((0..4).map(|_| result_message(false)));
        messages.push(tool_use_message(&["e"]));
        messages.push(result_message(false));
        // Keeping the last 6 would start the tail on the third result of the first call.
        let proposed = messages.len() - 6;
        assert!(first_tool_result(&messages[proposed]).is_some());
        let split = compaction_split_point(&messages, 2, proposed);
        assert_eq!(split, 2, "the split must move back to the tool call that owns the results");
        assert!(first_tool_result(&messages[split]).is_none());
    }

    #[test]
    fn compaction_split_keeps_a_split_that_starts_at_a_tool_call() {
        let mut messages = vec![Message::system("s"), Message::user("q"), tool_use_message(&["a"]), result_message(false)];
        messages.push(tool_use_message(&["b"]));
        messages.push(result_message(false));
        assert_eq!(compaction_split_point(&messages, 2, 4), 4);
    }

    /// Plays back a script in which any step may fail, and records every request's messages.
    struct ScriptedLlm {
        steps: Mutex<VecDeque<std::result::Result<LlmResponse, String>>>,
        requests: Mutex<Vec<Vec<Message>>>,
    }

    impl ScriptedLlm {
        fn new(steps: Vec<std::result::Result<LlmResponse, String>>) -> Arc<Self> {
            Arc::new(Self { steps: Mutex::new(steps.into()), requests: Mutex::new(Vec::new()) })
        }
    }

    #[async_trait]
    impl LlmProvider for ScriptedLlm {
        fn id(&self) -> &str { "scripted-llm" }
        fn kind(&self) -> &str { "mock" }
        fn default_model(&self) -> &str { "mock-model" }

        async fn list_models(&self) -> Result<Vec<crate::llm::types::ModelInfo>> { Ok(vec![]) }

        async fn chat_with(&self, _model: Option<&str>, messages: &[Message], _tools: &[ToolDefinition]) -> Result<LlmResponse> {
            self.requests.lock().unwrap().push(messages.to_vec());
            match self.steps.lock().unwrap().pop_front() {
                Some(Ok(response)) => Ok(response),
                Some(Err(e)) => Err(anyhow::anyhow!(e)),
                None => Err(anyhow::anyhow!("ScriptedLlm: no more steps")),
            }
        }
    }

    #[tokio::test]
    async fn empty_reply_after_tool_calls_is_recovered_by_synthesis() {
        let llm = MockLlm::new(vec![tool_call("my_tool"), text(""), text("The handler is in foo.rs.")]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "found something useful")], 5);
        let resp = agent.query("where is the handler?", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "The handler is in foo.rs.");
    }

    #[tokio::test]
    async fn stream_failure_is_not_reported_as_the_tool_limit() {
        let llm = ScriptedLlm::new(vec![
            Ok(tool_call("my_tool")),
            Err("OpenAI-compat API error 400: invalid message order".into()),
            Err("OpenAI-compat API error 400: invalid message order".into()),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "{\"raw\": \"json\"}")], 20);
        let resp = agent.query("q", &[], &[], None).await.unwrap();
        assert!(is_incomplete_answer(&resp.answer), "{}", resp.answer);
        assert!(resp.answer.contains("invalid message order"), "{}", resp.answer);
        assert!(!resp.answer.contains("tool calls"), "{}", resp.answer);
        assert!(!resp.answer.contains("raw"), "raw tool output must not be shown as the answer: {}", resp.answer);
    }

    #[tokio::test]
    async fn stream_failure_after_tool_calls_recovers_with_synthesis() {
        let llm = ScriptedLlm::new(vec![
            Ok(tool_call("my_tool")),
            Err("connection reset".into()),
            Ok(text("Recovered answer.")),
        ]);
        let agent = agent_with(llm, vec![MockTool::new("my_tool", "result")], 20);
        let resp = agent.query("q", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "Recovered answer.");
    }

    #[tokio::test]
    async fn search_nudge_never_separates_a_tool_call_from_its_result() {
        let search = || LlmResponse::ToolCalls {
            calls: vec![ToolCall { id: "tc_s".into(), name: "search_symbols".into(), input: serde_json::json!({ "query": "x" }), thought_signature: None }],
            preamble: String::new(),
            usage: Usage::default(),
        };
        let llm = ScriptedLlm::new(vec![Ok(search()), Ok(search()), Ok(search()), Ok(text("done"))]);
        let agent = agent_with(Arc::clone(&llm) as Arc<dyn LlmProvider>, vec![MockTool::new("search_symbols", "hits")], 10);
        let resp = agent.query("find x", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "done");

        let requests = llm.requests.lock().unwrap();
        let last = requests.last().expect("at least one request");
        for (i, message) in last.iter().enumerate() {
            let MessageContent::Parts(parts) = &message.content else { continue };
            if parts.iter().any(|p| matches!(p, ContentPart::ToolUse { .. })) {
                let next = last.get(i + 1).expect("a tool call must be followed by its result");
                assert!(first_tool_result(next).is_some(), "message after a tool call must be its result, got {:?}", next.content);
            }
        }
        let nudged = last.iter().filter_map(first_tool_result).any(|(_, c)| c.contains("Note from Harvest"));
        assert!(nudged, "the nudge should be attached to a tool result");
        assert!(
            !last.iter().any(|m| matches!(&m.content, MessageContent::Text(t) if t.contains("searched")
                || t.contains("Note from Harvest"))),
            "the nudge must not be sent as a user message",
        );
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
                usage: Usage::default(),
            },
            // Synthesis returns ToolCalls with preamble
            LlmResponse::ToolCalls {
                calls: vec![],
                preamble: "Here's what I found from the tools.".into(),
                usage: Usage::default(),
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

    #[test]
    fn build_synthesis_prompt_uniform_does_not_mention_variation() {
        let p = build_synthesis_prompt("prefix", "findings", true);
        assert!(!p.contains("differ"));
        assert!(!p.contains("first sentence"));
        assert!(!p.contains("blanket yes or no"));
    }

    #[test]
    fn build_synthesis_prompt_non_uniform_leads_with_variation() {
        let p = build_synthesis_prompt("prefix", "findings", false);
        assert!(p.contains("differ"));
        assert!(p.contains("first sentence"));
        assert!(p.contains("blanket yes or no"));
    }

    #[test]
    fn build_synthesis_prompt_always_includes_consequences_nudge() {
        let uniform = build_synthesis_prompt("prefix", "findings", true);
        let non_uniform = build_synthesis_prompt("prefix", "findings", false);
        assert!(uniform.contains("what happens when that capability is required"));
        assert!(non_uniform.contains("what happens when that capability is required"));
    }

    #[test]
    fn build_synthesis_prompt_includes_prefix_and_findings() {
        let p = build_synthesis_prompt("my prefix text", "my findings text", true);
        assert!(p.contains("my prefix text"));
        assert!(p.contains("my findings text"));
    }

    #[test]
    fn with_thresholds_sets_all_thresholds() {
        let llm = MockLlm::new(vec![text("done")]);
        let agent = Agent::new(llm, vec![], 5)
            .with_thresholds(0.6, 0.9, 0.3, 0.97, 3, 0.85, 6, 0.72);
        assert!((agent.fast_path_threshold - 0.6).abs() < f64::EPSILON);
        assert!((agent.early_synthesis_threshold - 0.9).abs() < f64::EPSILON);
        assert!((agent.relevance_threshold - 0.3).abs() < f64::EPSILON);
        assert!((agent.early_synthesis_research_threshold - 0.97).abs() < f64::EPSILON);
        assert_eq!(agent.relevance_preserve_recent, 3);
        assert!((agent.early_synthesis_coverage_threshold - 0.85).abs() < f64::EPSILON);
        assert_eq!(agent.early_synthesis_min_iterations_first_turn, 6);
        assert!((agent.early_synthesis_uniform_threshold - 0.72).abs() < f64::EPSILON);
    }

    #[test]
    fn default_thresholds_are_set() {
        let llm = MockLlm::new(vec![text("done")]);
        let agent = Agent::new(llm, vec![], 5);
        assert!((agent.fast_path_threshold - 0.7).abs() < f64::EPSILON);
        assert!((agent.early_synthesis_threshold - 0.85).abs() < f64::EPSILON);
        assert!((agent.relevance_threshold - 0.4).abs() < f64::EPSILON);
        assert!((agent.early_synthesis_research_threshold - 0.95).abs() < f64::EPSILON);
        assert_eq!(agent.relevance_preserve_recent, 2);
        assert!((agent.early_synthesis_coverage_threshold - 0.8).abs() < f64::EPSILON);
        assert_eq!(agent.early_synthesis_min_iterations_first_turn, 5);
        assert!((agent.early_synthesis_uniform_threshold - 0.7).abs() < f64::EPSILON);
    }

    fn mock_system_one_client(server_url: &str) -> Arc<crate::llm::system_one::SystemOneClient> {
        let cfg = crate::config::SystemOneConfig {
            endpoint: server_url.to_string(),
            api_key: "test-key".to_string(),
            model: "test-model".to_string(),
            timeout_secs: 10,
            user_provided_key: false,
            intent: crate::config::SystemOneThresholds::default(),
            model_routing: crate::config::SystemOneModelRouting::default(),
            tool_filter: crate::config::SystemOneThresholds::default(),
            compaction: crate::config::SystemOneThresholds::default(),
            parallel_research: crate::config::SystemOneThresholds::default(),
            prompt_sections: crate::config::SystemOneThresholds::default(),
            fast_path: 0.7,
            early_synthesis: 0.85,
            early_synthesis_research: 0.95,
            relevance: 0.4,
            relevance_preserve_recent: 2,
            early_synthesis_coverage: 0.8,
            early_synthesis_min_iterations_first_turn: 5,
            early_synthesis_uniform: 0.7,
            early_synthesis_capability_gate: 0.8,
            next_action_confidence: 0.5,
        };
        crate::llm::system_one::SystemOneClient::from_config(&cfg)
    }

    fn agent_with_system_one(
        llm: Arc<dyn LlmProvider>,
        tools: Vec<Box<dyn Tool>>,
        max: usize,
        so: Arc<crate::llm::system_one::SystemOneClient>,
    ) -> Agent {
        Agent::new(llm, tools, max)
            .with_system_one(Some(so))
            .with_thresholds(0.7, 0.85, 0.4, 0.95, 2, 0.8, 5, 0.7)
    }

    #[tokio::test]
    async fn early_synthesis_skipped_on_first_turn_research_before_min_iterations() {
        let server = httpmock::prelude::MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "enough_context": { "type": "noul", "noul": 0.99 },
                    "all_variants_examined": { "type": "noul", "noul": 0.99 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "relevance": { "type": "noul", "noul": 0.9 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        let so = mock_system_one_client(&server.base_url());
        let llm = MockLlm::new(vec![
            tool_call("t1"), tool_call("t2"), tool_call("t3"), tool_call("t4"),
            text("final answer"),
        ]);
        let agent = agent_with_system_one(llm, vec![MockTool::new("t1", "r1"), MockTool::new("t2", "r2"), MockTool::new("t3", "r3"), MockTool::new("t4", "r4")], 10, so);
        let resp = agent.query("how does X work?", &[], &[], None).await.unwrap();
        assert!(resp.tool_calls_made >= 4, "first-turn research should not synthesize before min_iterations (got {} calls)", resp.tool_calls_made);
    }

    #[tokio::test]
    async fn early_synthesis_uses_higher_threshold_for_research() {
        let server = httpmock::prelude::MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "enough_context": { "type": "noul", "noul": 0.9 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "relevance": { "type": "noul", "noul": 0.9 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        let so = mock_system_one_client(&server.base_url());
        let llm = MockLlm::new(vec![
            tool_call("t1"), tool_call("t2"), tool_call("t3"),
            text("synthesized"), text("fallback"),
        ]);
        let history = vec![HistoryMessage { role: "user".into(), text: "previous question".into(), attachments: None },
                          HistoryMessage { role: "assistant".into(), text: "previous answer".into(), attachments: None }];
        let agent = agent_with_system_one(llm, vec![MockTool::new("t1", "r1"), MockTool::new("t2", "r2"), MockTool::new("t3", "r3")], 10, so);
        let resp = agent.query("how does X work?", &history, &[], None).await.unwrap();
        assert!(resp.tool_calls_made >= 3, "research with 0.9 confidence (< 0.95 threshold) should not synthesize early (got {} calls)", resp.tool_calls_made);
    }

    #[tokio::test]
    async fn early_synthesis_requires_coverage_for_research() {
        let server = httpmock::prelude::MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("enough_context");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "enough_context": { "type": "noul", "noul": 0.99 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("all_variants_examined");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "all_variants_examined": { "type": "noul", "noul": 0.2 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "relevance": { "type": "noul", "noul": 0.9 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        let so = mock_system_one_client(&server.base_url());
        let llm = MockLlm::new(vec![
            tool_call("t1"), tool_call("t2"), tool_call("t3"),
            text("synthesized"), text("fallback"),
        ]);
        let history = vec![HistoryMessage { role: "user".into(), text: "previous".into(), attachments: None },
                          HistoryMessage { role: "assistant".into(), text: "answer".into(), attachments: None }];
        let agent = agent_with_system_one(llm, vec![MockTool::new("t1", "r1"), MockTool::new("t2", "r2"), MockTool::new("t3", "r3")], 10, so);
        let resp = agent.query("does X support Y?", &history, &[], None).await.unwrap();
        assert!(resp.tool_calls_made >= 3, "research should not synthesize when coverage is low (got {} calls)", resp.tool_calls_made);
    }

    fn next_action_server(choice: &str, confidence: f64) -> httpmock::MockServer {
        let server = httpmock::prelude::MockServer::start();
        let body = serde_json::json!({
            "model": "test-model",
            "answers": {
                "next_action": {
                    "type": "choice",
                    "choice": choice,
                    "probabilities": {},
                    "confidence": confidence
                }
            },
            "usage": { "input_tokens": 0, "output_tokens": 0 }
        });
        server.mock(|when, then| {
            when.method("POST").path("/").body_includes("next_action");
            then.status(200).json_body(body);
        });
        server
    }

    fn router_agent(server: &httpmock::MockServer) -> Arc<Agent> {
        let llm = MockLlm::new(vec![
            LlmResponse::ToolCalls {
                calls: vec![tool_call_obj("search_symbols", serde_json::json!({ "query": "alpha" }))],
                preamble: String::new(),
                usage: Usage::default(),
            },
            text("answer"),
        ]);
        let so = Arc::new(SystemOneClient::new(
            &crate::config::SystemOneConfig { endpoint: server.url("/"), api_key: "k".into(), model: "m".into(), ..Default::default() },
            "k".into(),
        ));
        Arc::new(
            Agent::new(llm, vec![MockTool::new("search_symbols", "found alpha")], 6)
                .with_system_one(Some(so))
                .with_thresholds(0.7, 0.5, 0.4, 0.5, 2, 0.5, 1, 0.5)
                .with_capability_gate_threshold(0.5)
                .with_next_action_confidence(0.5),
        )
    }

    #[tokio::test]
    async fn a_confident_synthesis_route_does_not_veto_synthesis() {
        let server = next_action_server("synthesize", 0.95);
        let agent = router_agent(&server);
        let resp = agent.query("how does alpha work?", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "answer");
        assert!(resp.tool_calls_made >= 1);
    }

    #[tokio::test]
    async fn a_confident_evidence_route_defers_synthesis() {
        let server = next_action_server("get_evidence_pack", 0.95);
        let agent = router_agent(&server);
        let resp = agent.query("how does alpha work?", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "answer");
    }

    #[tokio::test]
    async fn a_low_confidence_route_is_ignored() {
        let server = next_action_server("get_evidence_pack", 0.01);
        let agent = router_agent(&server);
        let resp = agent.query("how does alpha work?", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "answer");
    }

    fn uniformity_server(consistent: f64) -> httpmock::MockServer {
        let server = httpmock::prelude::MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("enough_context");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "enough_context": { "type": "noul", "noul": 0.99 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("all_variants_examined");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "all_variants_examined": { "type": "noul", "noul": 0.99 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("capability_gate_located");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "capability_gate_located": { "type": "noul", "noul": 0.95 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("findings_consistent");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "findings_consistent": { "type": "noul", "noul": consistent }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "relevance": { "type": "noul", "noul": 0.9 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server
    }

    #[tokio::test]
    async fn early_synthesis_deferred_when_capability_gate_not_located() {
        let server = capability_gate_blocked_server();
        let so = mock_system_one_client(&server.base_url());
        let llm = RecordingLlm::new(vec![
            tool_call("t1"), tool_call("t2"), tool_call("t3"),
            text("synthesized"), text("fallback"),
        ]);
        let history = vec![HistoryMessage { role: "user".into(), text: "previous".into(), attachments: None },
                          HistoryMessage { role: "assistant".into(), text: "answer".into(), attachments: None }];
        let agent = agent_with_system_one(llm.clone(), vec![MockTool::new("t1", "r1"), MockTool::new("t2", "r2"), MockTool::new("t3", "r3")], 10, so);
        agent.query("does X support Y?", &history, &[], None).await.unwrap();
        assert!(
            !llm.requests().iter().any(|r| r.contains("provide a complete answer")),
            "early synthesis must not fire before the capability gate is located"
        );
    }

    fn capability_gate_blocked_server() -> httpmock::MockServer {
        let server = httpmock::prelude::MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("enough_context");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "enough_context": { "type": "noul", "noul": 0.99 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("all_variants_examined");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "all_variants_examined": { "type": "noul", "noul": 0.99 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("capability_gate_located");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "capability_gate_located": { "type": "noul", "noul": 0.1 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "relevance": { "type": "noul", "noul": 0.9 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server
    }

    #[tokio::test]
    async fn early_synthesis_prompt_asks_for_variation_when_findings_differ() {
        let server = uniformity_server(0.1);
        let so = mock_system_one_client(&server.base_url());
        let llm = RecordingLlm::new(vec![
            tool_call("t1"), tool_call("t2"), tool_call("t3"),
            text("synthesized"), text("fallback"),
        ]);
        let history = vec![HistoryMessage { role: "user".into(), text: "previous".into(), attachments: None },
                          HistoryMessage { role: "assistant".into(), text: "answer".into(), attachments: None }];
        let agent = agent_with_system_one(llm.clone(), vec![MockTool::new("t1", "r1"), MockTool::new("t2", "r2"), MockTool::new("t3", "r3")], 10, so);
        agent.query("does X support Y?", &history, &[], None).await.unwrap();
        let synthesis = llm.requests().into_iter()
            .find(|r| r.contains("provide a complete answer"))
            .expect("early synthesis should have issued a synthesis prompt");
        assert!(synthesis.contains("blanket yes or no"), "non-uniform synthesis prompt should ask for variation");
        assert!(synthesis.contains("what happens when that capability is required"));
    }

    #[tokio::test]
    async fn early_synthesis_prompt_omits_variation_when_findings_uniform() {
        let server = uniformity_server(0.99);
        let so = mock_system_one_client(&server.base_url());
        let llm = RecordingLlm::new(vec![
            tool_call("t1"), tool_call("t2"), tool_call("t3"),
            text("synthesized"), text("fallback"),
        ]);
        let history = vec![HistoryMessage { role: "user".into(), text: "previous".into(), attachments: None },
                          HistoryMessage { role: "assistant".into(), text: "answer".into(), attachments: None }];
        let agent = agent_with_system_one(llm.clone(), vec![MockTool::new("t1", "r1"), MockTool::new("t2", "r2"), MockTool::new("t3", "r3")], 10, so);
        agent.query("does X support Y?", &history, &[], None).await.unwrap();
        let synthesis = llm.requests().into_iter()
            .find(|r| r.contains("provide a complete answer"))
            .expect("early synthesis should have issued a synthesis prompt");
        assert!(!synthesis.contains("blanket yes or no"), "uniform synthesis prompt should not ask for variation");
        assert!(synthesis.contains("what happens when that capability is required"));
    }

    #[tokio::test]
    async fn uniformity_gate_not_consulted_when_coverage_incomplete() {
        let server = httpmock::prelude::MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("enough_context");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "enough_context": { "type": "noul", "noul": 0.99 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("all_variants_examined");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "all_variants_examined": { "type": "noul", "noul": 0.1 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        let uniformity_mock = server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("findings_consistent");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "findings_consistent": { "type": "noul", "noul": 0.99 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "relevance": { "type": "noul", "noul": 0.9 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        let so = mock_system_one_client(&server.base_url());
        let llm = MockLlm::new(vec![
            tool_call("t1"), tool_call("t2"), tool_call("t3"),
            text("fallback"),
        ]);
        let history = vec![HistoryMessage { role: "user".into(), text: "previous".into(), attachments: None },
                          HistoryMessage { role: "assistant".into(), text: "answer".into(), attachments: None }];
        let agent = agent_with_system_one(llm, vec![MockTool::new("t1", "r1"), MockTool::new("t2", "r2"), MockTool::new("t3", "r3")], 10, so);
        let resp = agent.query("does X support Y?", &history, &[], None).await.unwrap();
        assert_eq!(resp.answer, "fallback");
        assert_eq!(uniformity_mock.calls(), 0, "uniformity must not be checked when coverage is incomplete");
    }

    #[tokio::test]
    async fn uniformity_check_failure_falls_back_to_non_uniform_prompt() {
        let server = httpmock::prelude::MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("enough_context");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "enough_context": { "type": "noul", "noul": 0.99 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("all_variants_examined");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "all_variants_examined": { "type": "noul", "noul": 0.99 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("capability_gate_located");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "capability_gate_located": { "type": "noul", "noul": 0.95 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("findings_consistent");
            then.status(500);
        });
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "relevance": { "type": "noul", "noul": 0.9 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        let so = mock_system_one_client(&server.base_url());
        let llm = RecordingLlm::new(vec![
            tool_call("t1"), tool_call("t2"), tool_call("t3"),
            text("synthesized"), text("fallback"),
        ]);
        let history = vec![HistoryMessage { role: "user".into(), text: "previous".into(), attachments: None },
                          HistoryMessage { role: "assistant".into(), text: "answer".into(), attachments: None }];
        let agent = agent_with_system_one(llm.clone(), vec![MockTool::new("t1", "r1"), MockTool::new("t2", "r2"), MockTool::new("t3", "r3")], 10, so);
        agent.query("does X support Y?", &history, &[], None).await.unwrap();
        let synthesis = llm.requests().into_iter()
            .find(|r| r.contains("provide a complete answer"))
            .expect("early synthesis should have issued a synthesis prompt");
        assert!(synthesis.contains("blanket yes or no"), "a failed uniformity check should fall back to the non-uniform prompt");
    }

    #[tokio::test]
    async fn max_iterations_synthesis_asks_for_variation_without_system_one() {
        let llm = RecordingLlm::new(vec![
            tool_call("t1"), tool_call("t2"),
            text("synthesized at cap"),
        ]);
        let agent = Agent::new(llm.clone(), vec![MockTool::new("t1", "r1"), MockTool::new("t2", "r2")], 2);
        let resp = agent.query("does X support Y?", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "synthesized at cap");
        let synthesis = llm.requests().into_iter()
            .find(|r| r.contains("maximum number of tool calls"))
            .expect("max-iterations path should have issued a synthesis prompt");
        assert!(synthesis.contains("blanket yes or no"), "max-iterations fallback should default to the non-uniform prompt");
        assert!(synthesis.contains("what happens when that capability is required"));
    }

    #[tokio::test]
    async fn max_iterations_synthesis_omits_variation_when_system_one_says_uniform() {
        let server = httpmock::prelude::MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("findings_consistent");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "findings_consistent": { "type": "noul", "noul": 0.99 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {},
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        let so = mock_system_one_client(&server.base_url());
        let llm = RecordingLlm::new(vec![
            tool_call("t1"), tool_call("t2"),
            text("synthesized at cap"),
        ]);
        let agent = Agent::new(llm.clone(), vec![MockTool::new("t1", "r1"), MockTool::new("t2", "r2")], 2)
            .with_system_one(Some(so));
        let resp = agent.query("does X support Y?", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "synthesized at cap");
        let synthesis = llm.requests().into_iter()
            .find(|r| r.contains("maximum number of tool calls"))
            .expect("max-iterations path should have issued a synthesis prompt");
        assert!(!synthesis.contains("blanket yes or no"), "uniform findings should use the uniform max-iterations prompt");
    }

    #[tokio::test]
    async fn relevance_scoring_sees_the_question_and_survives_multibyte_results() {
        let server = httpmock::prelude::MockServer::start();
        let relevance_with_goal = server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("\"relevance\"")
                .body_includes("which drivers support HA");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": { "relevance": { "type": "noul", "noul": 0.05 } },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {},
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        let so = mock_system_one_client(&server.base_url());
        // One ASCII byte then 3-byte characters, so byte offsets 200 and 2000 fall mid-character.
        let multibyte = format!("x{}", "…".repeat(1500));
        let llm = MockLlm::new(vec![
            tool_call("t1"), tool_call("t2"), tool_call("t3"), tool_call("t4"),
            text("done"),
        ]);
        let tools: Vec<Box<dyn Tool>> = (1..=4)
            .map(|i| MockTool::new(&format!("t{i}"), &multibyte) as Box<dyn Tool>)
            .collect();
        let agent = Agent::new(llm, tools, 10)
            .with_system_one(Some(so))
            .with_thresholds(0.7, 0.99, 0.4, 0.99, 2, 0.99, 99, 0.99);
        let resp = agent.query("which drivers support HA?", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "done");
        assert!(relevance_with_goal.calls() >= 1, "relevance scoring must be asked about the user's question, not an empty goal");
    }

    #[tokio::test]
    async fn relevance_loop_skips_recent_results_when_preserve_recent_set() {
        let server = httpmock::prelude::MockServer::start();
        let relevance_mock = server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("\"relevance\"");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "relevance": { "type": "noul", "noul": 0.05 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {},
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        let so = mock_system_one_client(&server.base_url());
        let llm = MockLlm::new(vec![
            tool_call("t1"), tool_call("t2"), tool_call("t3"), tool_call("t4"),
            text("done"),
        ]);
        let agent = Agent::new(llm, vec![MockTool::new("t1", "result-1"), MockTool::new("t2", "result-2"), MockTool::new("t3", "result-3"), MockTool::new("t4", "result-4")], 10)
            .with_system_one(Some(so))
            .with_thresholds(0.7, 0.99, 0.4, 0.99, 2, 0.99, 99, 0.99);
        let resp = agent.query("how does X work?", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "done");
        let hits_with_preserve = relevance_mock.calls();
        assert!(hits_with_preserve <= 6, "preserve_recent=2 should score at most 6 old results (got {} hits)", hits_with_preserve);
        assert!(hits_with_preserve >= 3, "at least 3 old results should be scored (got {} hits)", hits_with_preserve);

        let server2 = httpmock::prelude::MockServer::start();
        let relevance_mock2 = server2.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("\"relevance\"");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "relevance": { "type": "noul", "noul": 0.05 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server2.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {},
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        let so2 = mock_system_one_client(&server2.base_url());
        let llm2 = MockLlm::new(vec![
            tool_call("t1"), tool_call("t2"), tool_call("t3"), tool_call("t4"),
            text("done"),
        ]);
        let agent2 = Agent::new(llm2, vec![MockTool::new("t1", "result-1"), MockTool::new("t2", "result-2"), MockTool::new("t3", "result-3"), MockTool::new("t4", "result-4")], 10)
            .with_system_one(Some(so2))
            .with_thresholds(0.7, 0.99, 0.4, 0.99, 0, 0.99, 99, 0.99);
        let resp2 = agent2.query("how does X work?", &[], &[], None).await.unwrap();
        assert_eq!(resp2.answer, "done");
        let hits_without_preserve = relevance_mock2.calls();
        assert!(hits_without_preserve > hits_with_preserve, "preserve_recent=2 ({} hits) should score fewer than preserve_recent=0 ({} hits)", hits_with_preserve, hits_without_preserve);
    }

    #[tokio::test]
    async fn relevance_loop_preserves_all_when_fewer_than_preserve_recent() {
        let server = httpmock::prelude::MockServer::start();
        let relevance_mock = server.mock(|when, then| {
            when.method("POST").path("/")
                .body_includes("\"relevance\"");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {
                    "relevance": { "type": "noul", "noul": 0.05 }
                },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(serde_json::json!({
                "model": "test-model",
                "answers": {},
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }));
        });
        let so = mock_system_one_client(&server.base_url());
        let llm = MockLlm::new(vec![
            tool_call("t1"),
            text("done"),
        ]);
        let agent = Agent::new(llm, vec![MockTool::new("t1", "result-1")], 10)
            .with_system_one(Some(so))
            .with_thresholds(0.7, 0.99, 0.4, 0.99, 2, 0.99, 99, 0.99);
        let resp = agent.query("how does X work?", &[], &[], None).await.unwrap();
        assert_eq!(resp.answer, "done");
        assert_eq!(relevance_mock.calls(), 0, "with 1 result and preserve_recent=2, no results should be relevance-scored");
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

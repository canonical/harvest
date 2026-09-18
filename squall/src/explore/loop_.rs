use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use super::prompt::system_prompt;
use super::tools::ExplorationTool;
use crate::llm::pricing::PricingTable;
use crate::llm::types::{ContentPart, LlmResponse, Message, MessageContent, Role, ToolDefinition, Usage};
use crate::llm::LlmProvider;

const FINISH_TOOL_NAME: &str = "finish_exploration";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Bug {
    pub severity: String,
    pub area: String,
    pub description: String,
    #[serde(default)]
    pub repro_steps: String,
    #[serde(default)]
    pub evidence: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Improvement {
    pub area: String,
    pub suggestion: String,
    #[serde(default)]
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinishExplorationPayload {
    pub quality_assessment: String,
    #[serde(default)]
    pub bugs_found: Vec<Bug>,
    #[serde(default)]
    pub improvements: Vec<Improvement>,
    pub summary: String,
}

impl FinishExplorationPayload {
    fn fallback() -> Self {
        Self {
            quality_assessment: "Exploration ended without the model submitting a final report.".to_string(),
            bugs_found: vec![],
            improvements: vec![],
            summary: "The agent hit the iteration limit, or made an unparsable finish_exploration call, before formally concluding. Consult the transcript for partial findings.".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallRecord {
    pub name: String,
    pub input: Value,
    pub result: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TranscriptEntry {
    pub iteration: usize,
    pub provider_id: String,
    pub model: String,
    pub assistant_text: String,
    pub tool_calls: Vec<ToolCallRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExplorationResult {
    pub objective: String,
    pub transcript: Vec<TranscriptEntry>,
    pub tool_calls_made: usize,
    pub total_usage: Usage,
    pub estimated_cost_microusd: i64,
    pub duration_ms: u64,
    pub hit_max_iterations: bool,
    pub report: FinishExplorationPayload,
}

fn finish_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: FINISH_TOOL_NAME.to_string(),
        description: "Call this once the exploration objective is complete, or you are unable to make further progress, to submit your final analysis. This ends the run.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "quality_assessment": {"type": "string", "description": "Overall verdict on the quality of what you exercised, with reasoning."},
                "bugs_found": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "severity": {"type": "string", "description": "one of: low, medium, high, critical"},
                            "area": {"type": "string"},
                            "description": {"type": "string"},
                            "repro_steps": {"type": "string"},
                            "evidence": {"type": "string"}
                        },
                        "required": ["severity", "area", "description"]
                    }
                },
                "improvements": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "area": {"type": "string"},
                            "suggestion": {"type": "string"},
                            "rationale": {"type": "string"}
                        },
                        "required": ["area", "suggestion"]
                    }
                },
                "summary": {"type": "string"}
            },
            "required": ["quality_assessment", "summary"]
        }),
    }
}

fn parse_finish_payload(input: Value) -> Result<FinishExplorationPayload> {
    Ok(serde_json::from_value(input)?)
}

pub async fn run(
    llm: Arc<dyn LlmProvider>,
    tools: Vec<Arc<dyn ExplorationTool>>,
    pricing: &PricingTable,
    objective: &str,
    max_iterations: usize,
) -> Result<ExplorationResult> {
    let started = Instant::now();

    let mut tool_defs: Vec<ToolDefinition> = tools.iter().map(|t| t.definition()).collect();
    tool_defs.push(finish_tool_definition());
    let tool_map: HashMap<String, Arc<dyn ExplorationTool>> =
        tools.into_iter().map(|t| (t.definition().name, t)).collect();

    let mut messages = vec![
        Message::system(system_prompt(objective)),
        Message::user("Begin the exploration now."),
    ];
    let mut transcript = Vec::new();
    let mut total_usage = Usage::default();
    let mut estimated_cost_microusd = 0i64;
    let mut tool_calls_made = 0usize;
    let mut report: Option<FinishExplorationPayload> = None;

    for iteration in 0..max_iterations {
        let (response, used, usage) = llm.chat_routed(None, &messages, &tool_defs).await?;
        total_usage += usage.clone();
        estimated_cost_microusd += pricing.price_call(&usage, &used.kind, &used.model);

        match response {
            LlmResponse::Message { text, .. } => {
                messages.push(Message::assistant_text(text.clone()));
                transcript.push(TranscriptEntry {
                    iteration,
                    provider_id: used.provider_id,
                    model: used.model,
                    assistant_text: text,
                    tool_calls: vec![],
                });
                messages.push(Message::user(
                    "Call a tool to continue investigating, or call finish_exploration if the objective is complete.",
                ));
            }
            LlmResponse::ToolCalls { calls, preamble, .. } => {
                let mut records = Vec::new();
                let mut result_parts = Vec::new();
                let mut finished = None;

                for call in &calls {
                    if call.name == FINISH_TOOL_NAME {
                        finished = Some(parse_finish_payload(call.input.clone())?);
                        continue;
                    }
                    tool_calls_made += 1;
                    let outcome = match tool_map.get(call.name.as_str()) {
                        Some(tool) => tool.execute(call.input.clone()).await,
                        None => Err(anyhow::anyhow!("unknown tool: {}", call.name)),
                    };
                    let (content, is_error) = match outcome {
                        Ok(result) => (result, false),
                        Err(e) => (e.to_string(), true),
                    };
                    records.push(ToolCallRecord { name: call.name.clone(), input: call.input.clone(), result: content.clone(), is_error });
                    result_parts.push(ContentPart::ToolResult { tool_use_id: call.id.clone(), content, is_error });
                }

                let mut assistant_parts = Vec::new();
                if !preamble.is_empty() {
                    assistant_parts.push(ContentPart::Text { text: preamble.clone(), thought_signature: None });
                }
                for call in &calls {
                    assistant_parts.push(ContentPart::ToolUse {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        input: call.input.clone(),
                        thought_signature: call.thought_signature.clone(),
                    });
                }
                messages.push(Message { role: Role::Assistant, content: MessageContent::Parts(assistant_parts) });
                transcript.push(TranscriptEntry {
                    iteration,
                    provider_id: used.provider_id,
                    model: used.model,
                    assistant_text: preamble,
                    tool_calls: records,
                });

                if let Some(payload) = finished {
                    report = Some(payload);
                    break;
                }
                if !result_parts.is_empty() {
                    messages.push(Message { role: Role::User, content: MessageContent::Parts(result_parts) });
                }
            }
        }

        if report.is_some() {
            break;
        }
    }

    let hit_max_iterations = report.is_none();
    if report.is_none() {
        messages.push(Message::user(
            "You have reached the iteration limit. Call finish_exploration now with your findings so far.",
        ));
        let forced_defs = vec![finish_tool_definition()];
        if let Ok((response, used, usage)) = llm.chat_routed(None, &messages, &forced_defs).await {
            total_usage += usage.clone();
            estimated_cost_microusd += pricing.price_call(&usage, &used.kind, &used.model);
            if let LlmResponse::ToolCalls { calls, .. } = response {
                for call in calls {
                    if call.name == FINISH_TOOL_NAME {
                        if let Ok(payload) = parse_finish_payload(call.input) {
                            report = Some(payload);
                        }
                    }
                }
            }
        }
    }

    Ok(ExplorationResult {
        objective: objective.to_string(),
        transcript,
        tool_calls_made,
        total_usage,
        estimated_cost_microusd,
        duration_ms: started.elapsed().as_millis() as u64,
        hit_max_iterations,
        report: report.unwrap_or_else(FinishExplorationPayload::fallback),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::{ModelInfo, ToolCall};
    use anyhow::bail;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct ScriptedLlm {
        responses: Mutex<Vec<LlmResponse>>,
        received: Mutex<Vec<Vec<Message>>>,
    }

    impl ScriptedLlm {
        fn new(responses: Vec<LlmResponse>) -> Arc<Self> {
            Arc::new(Self { responses: Mutex::new(responses), received: Mutex::new(Vec::new()) })
        }

        fn calls(&self) -> Vec<Vec<Message>> {
            self.received.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LlmProvider for ScriptedLlm {
        fn id(&self) -> &str { "scripted" }
        fn kind(&self) -> &str { "scripted" }
        fn default_model(&self) -> &str { "scripted-model" }
        async fn list_models(&self) -> Result<Vec<ModelInfo>> { Ok(vec![]) }
        async fn chat_with(&self, _model: Option<&str>, messages: &[Message], _tools: &[ToolDefinition]) -> Result<LlmResponse> {
            self.received.lock().unwrap().push(messages.to_vec());
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                bail!("ScriptedLlm: no more scripted responses");
            }
            Ok(responses.remove(0))
        }
    }

    struct RecordingTool {
        name: &'static str,
        result: Result<String, String>,
        calls: Arc<Mutex<Vec<Value>>>,
    }

    #[async_trait]
    impl ExplorationTool for RecordingTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition { name: self.name.to_string(), description: "test tool".to_string(), parameters: json!({"type": "object"}) }
        }
        async fn execute(&self, params: Value) -> Result<String> {
            self.calls.lock().unwrap().push(params);
            match &self.result {
                Ok(s) => Ok(s.clone()),
                Err(e) => bail!("{e}"),
            }
        }
    }

    fn tool_call(id: &str, name: &str, input: Value) -> ToolCall {
        ToolCall { id: id.to_string(), name: name.to_string(), input, thought_signature: None }
    }

    fn finish_call(quality: &str, summary: &str) -> ToolCall {
        tool_call(
            "finish-1",
            FINISH_TOOL_NAME,
            json!({"quality_assessment": quality, "summary": summary}),
        )
    }

    fn no_pricing() -> PricingTable {
        PricingTable::from_configs(&[])
    }

    #[tokio::test]
    async fn immediate_finish_exploration_terminates_with_its_payload() {
        let llm = ScriptedLlm::new(vec![LlmResponse::ToolCalls {
            calls: vec![finish_call("looks solid", "nothing else to check")],
            preamble: String::new(),
            usage: Usage::default(),
        }]);
        let result = run(llm, vec![], &no_pricing(), "test the thing", 10).await.unwrap();
        assert_eq!(result.report.quality_assessment, "looks solid");
        assert_eq!(result.report.summary, "nothing else to check");
        assert!(!result.hit_max_iterations);
        assert_eq!(result.tool_calls_made, 0);
    }

    #[tokio::test]
    async fn dispatches_tool_call_then_finishes_on_next_turn() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let tool = Arc::new(RecordingTool { name: "create_project", result: Ok("{\"id\":\"p1\"}".into()), calls: calls.clone() });
        let llm = ScriptedLlm::new(vec![
            LlmResponse::ToolCalls {
                calls: vec![tool_call("c1", "create_project", json!({"name": "n"}))],
                preamble: "creating a project".into(),
                usage: Usage::default(),
            },
            LlmResponse::ToolCalls {
                calls: vec![finish_call("good", "done")],
                preamble: String::new(),
                usage: Usage::default(),
            },
        ]);
        let result = run(llm, vec![tool], &no_pricing(), "objective", 10).await.unwrap();
        assert_eq!(result.tool_calls_made, 1);
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(result.transcript[0].tool_calls[0].name, "create_project");
        assert!(!result.transcript[0].tool_calls[0].is_error);
        assert_eq!(result.report.summary, "done");
    }

    #[tokio::test]
    async fn tool_execution_error_is_recorded_and_loop_continues() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let tool = Arc::new(RecordingTool { name: "create_project", result: Err("boom".into()), calls });
        let llm = ScriptedLlm::new(vec![
            LlmResponse::ToolCalls {
                calls: vec![tool_call("c1", "create_project", json!({}))],
                preamble: String::new(),
                usage: Usage::default(),
            },
            LlmResponse::ToolCalls {
                calls: vec![finish_call("q", "s")],
                preamble: String::new(),
                usage: Usage::default(),
            },
        ]);
        let result = run(llm, vec![tool], &no_pricing(), "objective", 10).await.unwrap();
        assert!(result.transcript[0].tool_calls[0].is_error);
        assert!(result.transcript[0].tool_calls[0].result.contains("boom"));
    }

    #[tokio::test]
    async fn unknown_tool_name_is_recorded_as_error_without_panicking() {
        let llm = ScriptedLlm::new(vec![
            LlmResponse::ToolCalls {
                calls: vec![tool_call("c1", "does_not_exist", json!({}))],
                preamble: String::new(),
                usage: Usage::default(),
            },
            LlmResponse::ToolCalls {
                calls: vec![finish_call("q", "s")],
                preamble: String::new(),
                usage: Usage::default(),
            },
        ]);
        let result = run(llm, vec![], &no_pricing(), "objective", 10).await.unwrap();
        assert!(result.transcript[0].tool_calls[0].is_error);
        assert!(result.transcript[0].tool_calls[0].result.contains("unknown tool"));
    }

    #[tokio::test]
    async fn text_only_response_does_not_terminate_the_loop() {
        let llm = ScriptedLlm::new(vec![
            LlmResponse::Message { text: "thinking...".into(), usage: Usage::default() },
            LlmResponse::ToolCalls {
                calls: vec![finish_call("q", "s")],
                preamble: String::new(),
                usage: Usage::default(),
            },
        ]);
        let result = run(llm, vec![], &no_pricing(), "objective", 10).await.unwrap();
        assert_eq!(result.transcript[0].assistant_text, "thinking...");
        assert_eq!(result.report.summary, "s");
    }

    #[tokio::test]
    async fn exhausting_iterations_without_finish_call_sets_hit_max_iterations() {
        let llm = ScriptedLlm::new(vec![
            LlmResponse::Message { text: "still working".into(), usage: Usage::default() },
            LlmResponse::Message { text: "still working".into(), usage: Usage::default() },
        ]);
        let result = run(llm, vec![], &no_pricing(), "objective", 2).await.unwrap();
        assert!(result.hit_max_iterations);
        assert_eq!(result.report.quality_assessment, FinishExplorationPayload::fallback().quality_assessment);
    }

    #[tokio::test]
    async fn hitting_max_iterations_forces_a_synthesis_call_that_can_still_finish() {
        let llm = ScriptedLlm::new(vec![
            LlmResponse::Message { text: "still working".into(), usage: Usage::default() },
            LlmResponse::ToolCalls {
                calls: vec![finish_call("forced but complete", "wrapped up under pressure")],
                preamble: String::new(),
                usage: Usage::default(),
            },
        ]);
        let result = run(llm, vec![], &no_pricing(), "objective", 1).await.unwrap();
        assert!(result.hit_max_iterations);
        assert_eq!(result.report.quality_assessment, "forced but complete");
    }

    #[tokio::test]
    async fn first_call_always_includes_a_non_empty_user_turn() {
        let llm = ScriptedLlm::new(vec![LlmResponse::ToolCalls {
            calls: vec![finish_call("q", "s")],
            preamble: String::new(),
            usage: Usage::default(),
        }]);
        let llm_for_inspection = llm.clone();
        run(llm, vec![], &no_pricing(), "objective", 10).await.unwrap();

        let calls = llm_for_inspection.calls();
        let first_call_messages = &calls[0];
        assert!(
            first_call_messages.iter().any(|m| matches!(m.role, Role::User) && !matches!(&m.content, MessageContent::Text(t) if t.is_empty())),
            "first LLM call must include a non-empty user turn, or providers that fold system into a side channel (e.g. Gemini) see an empty contents array and reject the request",
        );
    }

    #[tokio::test]
    async fn usage_and_cost_accumulate_across_iterations() {
        let llm = ScriptedLlm::new(vec![
            LlmResponse::ToolCalls {
                calls: vec![finish_call("q", "s")],
                preamble: String::new(),
                usage: Usage { input_tokens: 100, output_tokens: 50, ..Default::default() },
            },
        ]);
        let result = run(llm, vec![], &no_pricing(), "objective", 10).await.unwrap();
        assert_eq!(result.total_usage.input_tokens, 100);
        assert_eq!(result.total_usage.output_tokens, 50);
    }
}

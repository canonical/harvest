use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::agent::IntentMode;
use crate::config::{LlmProviderConfig, SystemOneConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SystemOneQuestion {
    Choice {
        instructions: String,
        criteria: HashMap<String, String>,
    },
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
    Noul {
        instructions: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SystemOneAnswer {
    Choice {
        choice: String,
        probabilities: HashMap<String, f64>,
        confidence: f64,
    },
    Score {
        score: f64,
        probabilities: HashMap<String, f64>,
        confidence: f64,
    },
    Noul {
        noul: f64,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SystemOneUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SystemOneResponse {
    #[serde(default)]
    pub model: String,
    pub answers: HashMap<String, SystemOneAnswer>,
    #[serde(default)]
    pub usage: SystemOneUsage,
}

pub struct SystemOneClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextAction {
    GetEvidencePack,
    SearchSymbols,
    ReadSources,
    Synthesize,
    AskQuestion,
    Continue,
}

impl NextAction {
    pub fn key(self) -> &'static str {
        match self {
            NextAction::GetEvidencePack => "get_evidence_pack",
            NextAction::SearchSymbols => "search_symbols",
            NextAction::ReadSources => "read_sources",
            NextAction::Synthesize => "synthesize",
            NextAction::AskQuestion => "ask_question",
            NextAction::Continue => "continue",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "get_evidence_pack" => Some(NextAction::GetEvidencePack),
            "search_symbols" => Some(NextAction::SearchSymbols),
            "read_sources" => Some(NextAction::ReadSources),
            "synthesize" => Some(NextAction::Synthesize),
            "ask_question" => Some(NextAction::AskQuestion),
            "continue" => Some(NextAction::Continue),
            _ => None,
        }
    }

    pub fn criterion(self) -> &'static str {
        match self {
            NextAction::GetEvidencePack => "The evidence pack has not been read yet and would answer the question directly",
            NextAction::SearchSymbols => "Symbol names, signatures, or paths are still unknown and must be located first",
            NextAction::ReadSources => "Symbols are known but their bodies have not been read yet",
            NextAction::Synthesize => "Enough evidence has been gathered to answer",
            NextAction::AskQuestion => "The question is ambiguous and the user must choose a direction",
            NextAction::Continue => "Useful work remains but no single action dominates",
        }
    }
}

pub const NEXT_ACTION_KEY: &str = "next_action";
pub const DEFAULT_NEXT_ACTION_CONFIDENCE: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NextActionDecision {
    pub action: NextAction,
    pub confidence: f64,
    pub fallback: bool,
}

pub fn candidate_criteria(candidates: &[NextAction]) -> HashMap<String, String> {
    candidates
        .iter()
        .map(|c| (c.key().to_string(), c.criterion().to_string()))
        .collect()
}

pub fn resolve_next_action(
    answers: &HashMap<String, SystemOneAnswer>,
    candidates: &[NextAction],
    min_confidence: f64,
) -> NextActionDecision {
    let fallback = || NextActionDecision {
        action: candidates.first().copied().unwrap_or(NextAction::Continue),
        confidence: 0.0,
        fallback: true,
    };
    let Some(SystemOneAnswer::Choice { choice, confidence, .. }) = answers.get(NEXT_ACTION_KEY) else {
        return fallback();
    };
    let Some(action) = NextAction::from_key(choice) else {
        return fallback();
    };
    if !candidates.contains(&action) {
        return fallback();
    }
    if !confidence.is_finite() || *confidence < min_confidence {
        return NextActionDecision { action, confidence: *confidence, fallback: true };
    }
    NextActionDecision { action, confidence: *confidence, fallback: false }
}

impl SystemOneClient {
    pub fn new(config: &SystemOneConfig, api_key: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(config.timeout_secs))
                .build()
                .unwrap_or_default(),
            endpoint: config.endpoint.clone(),
            api_key,
            model: config.model.clone(),
        }
    }

    pub fn from_config(config: &SystemOneConfig) -> Arc<Self> {
        Arc::new(Self::new(config, config.api_key.clone()))
    }

    pub async fn resolve_for_user(
        config: &SystemOneConfig,
        llm_configs: &[LlmProviderConfig],
        user_key_store: &Option<Arc<crate::auth::user_keys::UserKeyStore>>,
        user_id: &str,
    ) -> Option<Arc<Self>> {
        if !config.api_key.is_empty() {
            return Some(Self::from_config(config));
        }

        let key_provider_id = config.key_provider_id(llm_configs);

        if let Some(store) = user_key_store {
            if let Ok(Some(key)) = store.get(user_id, &key_provider_id).await {
                if !key.is_empty() {
                    return Some(Arc::new(Self::new(config, key)));
                }
            }
        }

        tracing::info!(
            key_provider_id = %key_provider_id,
            "system-one key not available — falling back to heuristics"
        );
        None
    }

    pub async fn evaluate(
        &self,
        state: &str,
        questions: HashMap<String, SystemOneQuestion>,
    ) -> Result<SystemOneResponse> {
        let body = serde_json::json!({
            "model": self.model,
            "state": state,
            "questions": questions,
        });

        let resp = self
            .http
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if status.as_u16() == 429 || status.as_u16() == 529 {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "system-one rate limited ({}): {}",
                status,
                text
            ));
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("system-one error ({}): {}", status, text));
        }

        let response: SystemOneResponse = resp.json().await?;
        Ok(response)
    }

    pub async fn classify_intent(
        &self,
        user_query: &str,
        history: &str,
    ) -> Result<(IntentMode, f64)> {
        let mut questions = HashMap::new();
        questions.insert(
            "intent".to_string(),
            SystemOneQuestion::Choice {
                instructions: "The primary intent of this user message".into(),
                criteria: HashMap::from([
                    ("conversational".to_string(), "General question or chat, no code or infrastructure action needed".into()),
                    ("research".to_string(), "Needs to search or analyze code or repositories".into()),
                    ("action".to_string(), "Wants to run, deploy, provision, or modify infrastructure".into()),
                    ("hybrid".to_string(), "Both research and action in one request".into()),
                ]),
            },
        );
        questions.insert(
            "needs_tools".to_string(),
            SystemOneQuestion::Noul {
                instructions: "Answering this message requires calling tools beyond ask_user".into(),
            },
        );

        let state = if history.is_empty() {
            user_query.to_string()
        } else {
            format!("Recent conversation:\n{history}\n\nCurrent message:\n{user_query}")
        };

        let response = self.evaluate(&state, questions).await?;

        let mut intent = IntentMode::Research;
        let mut confidence = 0.0;

        if let Some(SystemOneAnswer::Choice {
            choice,
            confidence: c,
            ..
        }) = response.answers.get("intent")
        {
            intent = IntentMode::from_str(choice);
            confidence = *c;
        }

        if let Some(SystemOneAnswer::Noul { noul }) = response.answers.get("needs_tools") {
            if *noul < 0.3 {
                intent = IntentMode::Conversational;
            }
        }

        Ok((intent, confidence))
    }

    pub async fn route_model(
        &self,
        message: &str,
        history_summary: &str,
    ) -> Result<(ModelTier, f64)> {
        let mut questions = HashMap::new();
        questions.insert(
            "model_tier".to_string(),
            SystemOneQuestion::Choice {
                instructions: "Which model tier is appropriate for this turn?".into(),
                criteria: HashMap::from([
                    ("small".to_string(), "Simple factual, conversational, or lookup — no multi-step reasoning".into()),
                    ("medium".to_string(), "Moderate reasoning, single tool call, or straightforward code analysis".into()),
                    ("large".to_string(), "Multi-step reasoning, deployment design, complex code relationships".into()),
                ]),
            },
        );

        let state = if history_summary.is_empty() {
            message.to_string()
        } else {
            format!("Conversation summary:\n{history_summary}\n\nCurrent message:\n{message}")
        };

        let response = self.evaluate(&state, questions).await?;

        let mut tier = ModelTier::Large;
        let mut confidence = 0.0;

        if let Some(SystemOneAnswer::Choice {
            choice,
            confidence: c,
            ..
        }) = response.answers.get("model_tier")
        {
            tier = match choice.as_str() {
                "small" => ModelTier::Small,
                "medium" => ModelTier::Medium,
                _ => ModelTier::Large,
            };
            confidence = *c;
        }

        Ok((tier, confidence))
    }

    pub async fn filter_tool_calls(
        &self,
        tool_calls: &[(String, String)],
        history: &str,
    ) -> Result<Vec<bool>> {
        self.filter_tool_calls_with_threshold(tool_calls, history, 0.5).await
    }

    pub async fn filter_tool_calls_with_threshold(
        &self,
        tool_calls: &[(String, String)],
        history: &str,
        threshold: f64,
    ) -> Result<Vec<bool>> {
        if tool_calls.is_empty() {
            return Ok(Vec::new());
        }

        let mut questions = HashMap::new();
        for (i, (name, args)) in tool_calls.iter().enumerate() {
            questions.insert(
                format!("call_{i}_necessary"),
                SystemOneQuestion::Noul {
                    instructions: format!(
                        "This tool call ({name} with args {args}) is necessary to answer the user's question given the conversation history"
                    ),
                },
            );
        }

        let state = format!("Conversation history:\n{history}");

        let response = self.evaluate(&state, questions).await?;

        Ok(tool_calls
            .iter()
            .enumerate()
            .map(|(i, _)| {
                response
                    .answers
                    .get(&format!("call_{i}_necessary"))
                    .map(|a| {
                        if let SystemOneAnswer::Noul { noul } = a {
                            *noul >= threshold
                        } else {
                            true
                        }
                    })
                    .unwrap_or(true)
            })
            .collect())
    }

    pub async fn should_compact(
        &self,
        history: &str,
        current_query: &str,
    ) -> Result<(bool, f64)> {
        let mut questions = HashMap::new();
        questions.insert(
            "needs_compaction".to_string(),
            SystemOneQuestion::Noul {
                instructions:
                    "The conversation history is noisy enough that compaction would improve response quality"
                        .into(),
            },
        );

        let state = format!(
            "Conversation history:\n{history}\n\nCurrent question:\n{current_query}"
        );

        let response = self.evaluate(&state, questions).await?;

        if let Some(SystemOneAnswer::Noul { noul }) = response.answers.get("needs_compaction")
        {
            Ok((*noul >= 0.5, *noul))
        } else {
            Ok((true, 1.0))
        }
    }

    pub async fn should_parallel_research(
        &self,
        query: &str,
        history: &str,
    ) -> Result<(bool, usize)> {
        let mut questions = HashMap::new();
        questions.insert(
            "needs_parallel".to_string(),
            SystemOneQuestion::Noul {
                instructions:
                    "This question benefits from parallel research across multiple independent angles"
                        .into(),
            },
        );
        questions.insert(
            "breadth".to_string(),
            SystemOneQuestion::Score {
                instructions: "How many independent research angles does this question have?".into(),
                criteria: vec![
                    "One focused question — one angle suffices".into(),
                    "2-3 related but distinct angles".into(),
                    "4 or more truly independent aspects to investigate".into(),
                ],
            },
        );

        let state = if history.is_empty() {
            query.to_string()
        } else {
            format!("Conversation history:\n{history}\n\nCurrent question:\n{query}")
        };

        let response = self.evaluate(&state, questions).await?;

        let mut should = false;
        let mut count = 3;

        if let Some(SystemOneAnswer::Noul { noul }) = response.answers.get("needs_parallel") {
            should = *noul >= 0.5;
        }

        if let Some(SystemOneAnswer::Score { score, .. }) = response.answers.get("breadth") {
            count = (*score as usize).clamp(2, 6);
        }

        Ok((should, count))
    }

    pub async fn select_prompt_sections(
        &self,
        query: &str,
        collocate_enabled: bool,
    ) -> Result<PromptSectionFlags> {
        let mut questions = HashMap::new();
        questions.insert(
            "needs_citations".to_string(),
            SystemOneQuestion::Noul {
                instructions:
                    "This turn involves referencing specific source code that requires citation formatting"
                        .into(),
            },
        );
        questions.insert(
            "needs_mermaid".to_string(),
            SystemOneQuestion::Noul {
                instructions:
                    "This turn likely involves generating diagrams or visual graphs".into(),
            },
        );
        questions.insert(
            "needs_workflow".to_string(),
            SystemOneQuestion::Noul {
                instructions:
                    "This turn involves multi-step infrastructure deployment or provisioning"
                        .into(),
            },
        );
        questions.insert(
            "needs_cross_repo".to_string(),
            SystemOneQuestion::Noul {
                instructions:
                    "This turn involves cross-repository questions about how two or more repos relate"
                        .into(),
            },
        );
        questions.insert(
            "needs_test_infra".to_string(),
            SystemOneQuestion::Noul {
                instructions: "This turn involves finding test infrastructure".into(),
            },
        );

        let state = if collocate_enabled {
            format!("Collocate tools are available.\n\nUser message:\n{query}")
        } else {
            format!("User message:\n{query}")
        };

        let response = self.evaluate(&state, questions).await?;

        let get_noul = |key: &str| -> bool {
            response
                .answers
                .get(key)
                .map(|a| {
                    if let SystemOneAnswer::Noul { noul } = a {
                        *noul >= 0.5
                    } else {
                        true
                    }
                })
                .unwrap_or(true)
        };

        Ok(PromptSectionFlags {
            citations: get_noul("needs_citations"),
            mermaid: get_noul("needs_mermaid"),
            workflow: get_noul("needs_workflow"),
            cross_repo: get_noul("needs_cross_repo"),
            test_infra: get_noul("needs_test_infra"),
        })
    }

    pub async fn route_model_enhanced(
        &self,
        message: &str,
        history_summary: &str,
    ) -> Result<(ModelTier, f64, bool)> {
        let mut questions = HashMap::new();
        questions.insert(
            "model_tier".to_string(),
            SystemOneQuestion::Choice {
                instructions: "Which model tier is appropriate for this turn?".into(),
                criteria: HashMap::from([
                    ("small".to_string(), "Simple factual, conversational, or lookup — no multi-step reasoning".into()),
                    ("medium".to_string(), "Moderate reasoning, single tool call, or straightforward code analysis".into()),
                    ("large".to_string(), "Multi-step reasoning, deployment design, complex code relationships".into()),
                ]),
            },
        );
        questions.insert(
            "needs_deep_reasoning".to_string(),
            SystemOneQuestion::Noul {
                instructions: "This turn requires deep multi-step reasoning to decide what to do next".into(),
            },
        );

        let state = if history_summary.is_empty() {
            message.to_string()
        } else {
            format!("Conversation summary:\n{history_summary}\n\nCurrent message:\n{message}")
        };

        let response = self.evaluate(&state, questions).await?;

        let mut tier = ModelTier::Large;
        let mut confidence = 0.0;

        if let Some(SystemOneAnswer::Choice { choice, confidence: c, .. }) = response.answers.get("model_tier") {
            tier = match choice.as_str() {
                "small" => ModelTier::Small,
                "medium" => ModelTier::Medium,
                _ => ModelTier::Large,
            };
            confidence = *c;
        }

        let mut needs_deep = false;
        if let Some(SystemOneAnswer::Noul { noul }) = response.answers.get("needs_deep_reasoning") {
            needs_deep = *noul > 0.7;
            if needs_deep && tier == ModelTier::Small {
                tier = ModelTier::Medium;
            }
        }

        Ok((tier, confidence, needs_deep))
    }

    pub async fn score_tool_result_relevance(
        &self,
        tool_name: &str,
        tool_result: &str,
        goal: &str,
    ) -> Result<f64> {
        let truncated: String = tool_result.chars().take(2000).collect();
        let mut questions = HashMap::new();
        questions.insert(
            "relevance".to_string(),
            SystemOneQuestion::Noul {
                instructions: format!(
                    "This tool result ({tool_name}) is relevant to answering: {goal}\n\nTool result (truncated): {truncated}"
                ),
            },
        );

        let state = format!("Current goal: {goal}");
        let response = self.evaluate(&state, questions).await?;

        if let Some(SystemOneAnswer::Noul { noul }) = response.answers.get("relevance") {
            Ok(*noul)
        } else {
            Ok(1.0)
        }
    }

    pub async fn select_tools(
        &self,
        query: &str,
        tool_names: &[String],
    ) -> Result<Vec<String>> {
        if tool_names.len() <= 8 {
            return Ok(tool_names.to_vec());
        }

        let mut questions = HashMap::new();
        for name in tool_names {
            questions.insert(
                format!("use_{name}"),
                SystemOneQuestion::Noul {
                    instructions: format!("The tool '{name}' is likely to be needed to answer: {query}"),
                },
            );
        }

        let response = self.evaluate(query, questions).await?;

        let selected: Vec<String> = tool_names.iter()
            .filter(|name| {
                response.answers.get(&format!("use_{}", name))
                    .map(|a| {
                        if let SystemOneAnswer::Noul { noul } = a { *noul >= 0.4 } else { true }
                    })
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        if selected.is_empty() {
            Ok(tool_names.to_vec())
        } else {
            Ok(selected)
        }
    }

    pub async fn route_next_action(
        &self,
        goal: &str,
        progress: &str,
        candidates: &[NextAction],
        min_confidence: f64,
    ) -> Result<NextActionDecision> {
        if candidates.is_empty() {
            return Ok(NextActionDecision {
                action: NextAction::Continue,
                confidence: 0.0,
                fallback: true,
            });
        }
        let mut questions = HashMap::new();
        questions.insert(
            NEXT_ACTION_KEY.to_string(),
            SystemOneQuestion::Choice {
                instructions: format!(
                    "Choose the single most useful next action for this investigation.\n\nGoal: {goal}\n\nProgress so far:\n{progress}"
                ),
                criteria: candidate_criteria(candidates),
            },
        );

        let response = self.evaluate(goal, questions).await?;
        Ok(resolve_next_action(&response.answers, candidates, min_confidence))
    }

    pub async fn should_synthesize(
        &self,
        goal: &str,
        accumulated_findings: &str,
    ) -> Result<(bool, f64)> {
        let mut questions = HashMap::new();
        questions.insert(
            "enough_context".to_string(),
            SystemOneQuestion::Noul {
                instructions: format!(
                    "Enough information has been gathered to fully answer this question.\n\nQuestion: {goal}\n\nFindings so far: {accumulated_findings}"
                ),
            },
        );

        let response = self.evaluate(goal, questions).await?;

        if let Some(SystemOneAnswer::Noul { noul }) = response.answers.get("enough_context") {
            Ok((*noul >= 0.5, *noul))
        } else {
            Ok((false, 0.0))
        }
    }

    pub async fn should_synthesize_coverage(
        &self,
        goal: &str,
        accumulated_findings: &str,
    ) -> Result<(bool, f64)> {
        let mut questions = HashMap::new();
        questions.insert(
            "all_variants_examined".to_string(),
            SystemOneQuestion::Noul {
                instructions: format!(
                    "All relevant variants, drivers, or sub-components of the question have been examined — none were skipped.\n\nQuestion: {goal}\n\nFindings so far: {accumulated_findings}"
                ),
            },
        );

        let response = self.evaluate(goal, questions).await?;

        if let Some(SystemOneAnswer::Noul { noul }) = response.answers.get("all_variants_examined") {
            Ok((*noul >= 0.5, *noul))
        } else {
            Ok((false, 0.0))
        }
    }

    pub async fn should_synthesize_capability_gate(
        &self,
        goal: &str,
        accumulated_findings: &str,
    ) -> Result<(bool, f64)> {
        self.should_synthesize_capability_gate_with_threshold(goal, accumulated_findings, 0.5)
            .await
    }

    pub async fn should_synthesize_capability_gate_with_threshold(
        &self,
        goal: &str,
        accumulated_findings: &str,
        threshold: f64,
    ) -> Result<(bool, f64)> {
        let mut questions = HashMap::new();
        questions.insert(
            "capability_gate_located".to_string(),
            SystemOneQuestion::Noul {
                instructions: format!(
                    "The investigation located the declaration, constant, or registry entry that \
                     determines whether the requested capability is available, determined its value \
                     for each variant examined including values inherited from an ancestor, and \
                     located the code that enforces it. Answer 0.0 if only the implementing method \
                     was found, or if the per-variant values are unknown.\n\nQuestion: {goal}\n\nFindings so far: {accumulated_findings}"
                ),
            },
        );

        let response = self.evaluate(goal, questions).await?;

        if let Some(SystemOneAnswer::Noul { noul }) = response.answers.get("capability_gate_located") {
            Ok((*noul >= threshold, *noul))
        } else {
            Ok((false, 0.0))
        }
    }

    pub async fn should_synthesize_uniform(
        &self,
        goal: &str,
        accumulated_findings: &str,
    ) -> Result<(bool, f64)> {
        let mut questions = HashMap::new();
        questions.insert(
            "findings_consistent".to_string(),
            SystemOneQuestion::Noul {
                instructions: format!(
                    "The findings are consistent across every entity, component, driver, module, or variant examined — the answer is the same for all of them. Answer 0.0 if the findings differ in any way.\n\nQuestion: {goal}\n\nFindings so far: {accumulated_findings}"
                ),
            },
        );

        let response = self.evaluate(goal, questions).await?;

        if let Some(SystemOneAnswer::Noul { noul }) = response.answers.get("findings_consistent") {
            Ok((*noul >= 0.5, *noul))
        } else {
            Ok((false, 0.0))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTier {
    Small,
    Medium,
    Large,
}

#[derive(Debug, Clone)]
pub struct PromptSectionFlags {
    pub citations: bool,
    pub mermaid: bool,
    pub workflow: bool,
    pub cross_repo: bool,
    pub test_infra: bool,
}

impl Default for PromptSectionFlags {
    fn default() -> Self {
        Self {
            citations: true,
            mermaid: true,
            workflow: true,
            cross_repo: true,
            test_infra: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn mock_client(server: &MockServer) -> SystemOneClient {
        SystemOneClient {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
            endpoint: server.base_url(),
            api_key: "test-key".into(),
            model: "test-model".into(),
        }
    }

    fn so_response(answers: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "model": "test-model",
            "answers": answers,
            "usage": { "input_tokens": 0, "output_tokens": 0 },
        })
    }

    #[tokio::test]
    async fn score_tool_result_relevance_returns_noul_value() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "relevance": { "type": "noul", "noul": 0.85 }
            })));
        });

        let client = mock_client(&server);
        let score = client.score_tool_result_relevance("search_symbols", "found auth middleware", "find auth").await.unwrap();
        assert!((score - 0.85).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn score_tool_result_relevance_returns_one_on_missing_answer() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({})));
        });

        let client = mock_client(&server);
        let score = client.score_tool_result_relevance("tool", "result", "goal").await.unwrap();
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn select_tools_returns_all_when_few_tools() {
        let server = MockServer::start();
        let client = mock_client(&server);
        let tools = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let selected = client.select_tools("query", &tools).await.unwrap();
        assert_eq!(selected, tools);
    }

    #[tokio::test]
    async fn select_tools_filters_when_many_tools() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "use_a": { "type": "noul", "noul": 0.9 },
                "use_b": { "type": "noul", "noul": 0.1 },
                "use_c": { "type": "noul", "noul": 0.8 },
                "use_d": { "type": "noul", "noul": 0.05 },
                "use_e": { "type": "noul", "noul": 0.6 },
                "use_f": { "type": "noul", "noul": 0.3 },
                "use_g": { "type": "noul", "noul": 0.9 },
                "use_h": { "type": "noul", "noul": 0.2 },
                "use_i": { "type": "noul", "noul": 0.7 }
            })));
        });

        let client = mock_client(&server);
        let tools: Vec<String> = (0..9).map(|i| ((b'a' + i) as char).to_string()).collect();
        let selected = client.select_tools("query", &tools).await.unwrap();
        assert!(selected.contains(&"a".to_string()));
        assert!(selected.contains(&"c".to_string()));
        assert!(selected.contains(&"e".to_string()));
        assert!(selected.contains(&"g".to_string()));
        assert!(selected.contains(&"i".to_string()));
        assert!(!selected.contains(&"b".to_string()));
        assert!(!selected.contains(&"d".to_string()));
        assert!(!selected.contains(&"f".to_string()));
        assert!(!selected.contains(&"h".to_string()));
    }

    #[tokio::test]
    async fn select_tools_returns_all_when_selection_empty() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "use_a": { "type": "noul", "noul": 0.1 },
                "use_b": { "type": "noul", "noul": 0.1 },
                "use_c": { "type": "noul", "noul": 0.1 },
                "use_d": { "type": "noul", "noul": 0.1 },
                "use_e": { "type": "noul", "noul": 0.1 },
                "use_f": { "type": "noul", "noul": 0.1 },
                "use_g": { "type": "noul", "noul": 0.1 },
                "use_h": { "type": "noul", "noul": 0.1 },
                "use_i": { "type": "noul", "noul": 0.1 }
            })));
        });

        let client = mock_client(&server);
        let tools: Vec<String> = (0..9).map(|i| ((b'a' + i) as char).to_string()).collect();
        let selected = client.select_tools("query", &tools).await.unwrap();
        assert_eq!(selected.len(), tools.len());
    }

    #[tokio::test]
    async fn should_synthesize_returns_true_when_noul_high() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "enough_context": { "type": "noul", "noul": 0.9 }
            })));
        });

        let client = mock_client(&server);
        let (should, conf) = client.should_synthesize("find auth", "found auth in auth.rs").await.unwrap();
        assert!(should);
        assert!((conf - 0.9).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn should_synthesize_returns_false_when_noul_low() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "enough_context": { "type": "noul", "noul": 0.2 }
            })));
        });

        let client = mock_client(&server);
        let (should, _) = client.should_synthesize("find auth", "no results yet").await.unwrap();
        assert!(!should);
    }

    #[tokio::test]
    async fn route_model_enhanced_upgrades_small_to_medium_when_deep_reasoning() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "model_tier": { "type": "choice", "choice": "small", "probabilities": {}, "confidence": 0.8 },
                "needs_deep_reasoning": { "type": "noul", "noul": 0.9 }
            })));
        });

        let client = mock_client(&server);
        let (tier, _, needs_deep) = client.route_model_enhanced("complex query", "").await.unwrap();
        assert!(needs_deep);
        assert_eq!(tier, ModelTier::Medium);
    }

    #[tokio::test]
    async fn route_model_enhanced_keeps_small_when_no_deep_reasoning() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "model_tier": { "type": "choice", "choice": "small", "probabilities": {}, "confidence": 0.8 },
                "needs_deep_reasoning": { "type": "noul", "noul": 0.2 }
            })));
        });

        let client = mock_client(&server);
        let (tier, _, needs_deep) = client.route_model_enhanced("simple query", "").await.unwrap();
        assert!(!needs_deep);
        assert_eq!(tier, ModelTier::Small);
    }

    #[tokio::test]
    async fn filter_tool_calls_with_threshold_filters_below_threshold() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "call_0_necessary": { "type": "noul", "noul": 0.3 },
                "call_1_necessary": { "type": "noul", "noul": 0.8 }
            })));
        });

        let client = mock_client(&server);
        let calls = vec![("search".to_string(), "{}".to_string()), ("get_source".to_string(), "{}".to_string())];
        let keep = client.filter_tool_calls_with_threshold(&calls, "history", 0.5).await.unwrap();
        assert_eq!(keep, vec![false, true]);
    }

    #[tokio::test]
    async fn filter_tool_calls_with_threshold_keeps_all_at_low_threshold() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "call_0_necessary": { "type": "noul", "noul": 0.3 },
                "call_1_necessary": { "type": "noul", "noul": 0.4 }
            })));
        });

        let client = mock_client(&server);
        let calls = vec![("search".to_string(), "{}".to_string()), ("get_source".to_string(), "{}".to_string())];
        let keep = client.filter_tool_calls_with_threshold(&calls, "history", 0.2).await.unwrap();
        assert_eq!(keep, vec![true, true]);
    }

    #[tokio::test]
    async fn capability_gate_returns_true_when_noul_high() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "capability_gate_located": { "type": "noul", "noul": 0.9 }
            })));
        });

        let client = mock_client(&server);
        let (located, conf) = client.should_synthesize_capability_gate("does it support HA", "found the flag").await.unwrap();
        assert!(located);
        assert!((conf - 0.9).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn capability_gate_returns_false_when_noul_low() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "capability_gate_located": { "type": "noul", "noul": 0.2 }
            })));
        });

        let client = mock_client(&server);
        let (located, _) = client.should_synthesize_capability_gate("goal", "findings").await.unwrap();
        assert!(!located);
    }

    #[tokio::test]
    async fn capability_gate_returns_false_on_missing_answer() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({})));
        });

        let client = mock_client(&server);
        let (located, conf) = client.should_synthesize_capability_gate("goal", "findings").await.unwrap();
        assert!(!located);
        assert!((conf - 0.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn capability_gate_prompt_demands_declaration_values_and_enforcement() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.is_true(|req: &HttpMockRequest| {
                let body = req.body_string();
                body.contains("capability_gate_located")
                    && body.contains("declaration")
                    && body.contains("enforces")
            }).method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "capability_gate_located": { "type": "noul", "noul": 0.9 }
            })));
        });

        let client = mock_client(&server);
        let (located, _) = client.should_synthesize_capability_gate("goal", "findings").await.unwrap();
        assert!(located);
    }

    #[tokio::test]
    async fn capability_gate_threshold_is_configurable() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "capability_gate_located": { "type": "noul", "noul": 0.7 }
            })));
        });

        let client = mock_client(&server);
        assert!(client.should_synthesize_capability_gate_with_threshold("goal", "f", 0.6).await.unwrap().0);
        assert!(!client.should_synthesize_capability_gate_with_threshold("goal", "f", 0.8).await.unwrap().0);
    }

    #[tokio::test]
    async fn should_synthesize_coverage_returns_true_when_noul_high() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "all_variants_examined": { "type": "noul", "noul": 0.9 }
            })));
        });

        let client = mock_client(&server);
        let (covered, conf) = client.should_synthesize_coverage("does NetApp support HA", "NFS yes, iSCSI no").await.unwrap();
        assert!(covered);
        assert!((conf - 0.9).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn should_synthesize_coverage_returns_false_when_noul_low() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "all_variants_examined": { "type": "noul", "noul": 0.2 }
            })));
        });

        let client = mock_client(&server);
        let (covered, _) = client.should_synthesize_coverage("does NetApp support HA", "NFS yes").await.unwrap();
        assert!(!covered);
    }

    #[tokio::test]
    async fn should_synthesize_coverage_returns_false_on_missing_answer() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({})));
        });

        let client = mock_client(&server);
        let (covered, conf) = client.should_synthesize_coverage("goal", "findings").await.unwrap();
        assert!(!covered);
        assert!((conf - 0.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn should_synthesize_uniform_returns_true_when_noul_high() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "findings_consistent": { "type": "noul", "noul": 0.9 }
            })));
        });

        let client = mock_client(&server);
        let (uniform, conf) = client.should_synthesize_uniform("does anything differ", "all identical").await.unwrap();
        assert!(uniform);
        assert!((conf - 0.9).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn should_synthesize_uniform_returns_false_when_noul_low() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "findings_consistent": { "type": "noul", "noul": 0.2 }
            })));
        });

        let client = mock_client(&server);
        let (uniform, _) = client.should_synthesize_uniform("does anything differ", "NFS yes, iSCSI no").await.unwrap();
        assert!(!uniform);
    }

    #[tokio::test]
    async fn should_synthesize_uniform_returns_false_on_missing_answer() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({})));
        });

        let client = mock_client(&server);
        let (uniform, conf) = client.should_synthesize_uniform("goal", "findings").await.unwrap();
        assert!(!uniform);
        assert!((conf - 0.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn should_synthesize_uniform_asks_findings_consistent_question() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST").path("/");
            then.status(200).json_body(so_response(serde_json::json!({
                "findings_consistent": { "type": "noul", "noul": 1.0 }
            })));
        });

        let client = mock_client(&server);
        let (_, conf) = client.should_synthesize_uniform("goal", "findings").await.unwrap();
        assert!((conf - 1.0).abs() < f64::EPSILON);
        assert_eq!(mock.calls(), 1);
    }
    fn choice(answer: &str, confidence: f64) -> HashMap<String, SystemOneAnswer> {
        let mut m = HashMap::new();
        m.insert(
            NEXT_ACTION_KEY.to_string(),
            SystemOneAnswer::Choice {
                choice: answer.to_string(),
                probabilities: HashMap::new(),
                confidence,
            },
        );
        m
    }

    const CANDIDATES: [NextAction; 3] = [NextAction::GetEvidencePack, NextAction::SearchSymbols, NextAction::Synthesize];

    #[test]
    fn next_action_keys_round_trip() {
        for a in [
            NextAction::GetEvidencePack,
            NextAction::SearchSymbols,
            NextAction::ReadSources,
            NextAction::Synthesize,
            NextAction::AskQuestion,
            NextAction::Continue,
        ] {
            assert_eq!(NextAction::from_key(a.key()), Some(a), "key round trip failed for {a:?}");
        }
    }

    #[test]
    fn an_unknown_key_does_not_resolve() {
        assert_eq!(NextAction::from_key("teleport"), None);
    }

    #[test]
    fn a_valid_action_outside_the_candidate_set_is_rejected() {
        let d = resolve_next_action(&choice("read_sources", 0.9), &CANDIDATES, 0.5);
        assert_eq!(d.action, NextAction::GetEvidencePack, "read_sources was not offered, so it must not be chosen");
        assert!(d.fallback);
        assert_eq!(d.confidence, 0.0);
    }

    #[test]
    fn a_candidate_choice_is_honoured() {
        let d = resolve_next_action(&choice("search_symbols", 0.9), &CANDIDATES, 0.5);
        assert_eq!(d.action, NextAction::SearchSymbols);
        assert!((d.confidence - 0.9).abs() < 1e-9);
        assert!(!d.fallback);
    }

    #[test]
    fn a_low_confidence_choice_is_kept_but_flagged_as_fallback() {
        let d = resolve_next_action(&choice("synthesize", 0.2), &CANDIDATES, 0.5);
        assert_eq!(d.action, NextAction::Synthesize);
        assert!(d.fallback, "low confidence must be surfaced to the caller");
    }

    #[test]
    fn an_unknown_answer_falls_back_to_the_first_candidate() {
        let d = resolve_next_action(&choice("teleport", 0.99), &CANDIDATES, 0.5);
        assert_eq!(d.action, NextAction::GetEvidencePack);
        assert!(d.fallback);
        assert_eq!(d.confidence, 0.0);
    }

    #[test]
    fn a_non_finite_confidence_is_treated_as_low() {
        let d = resolve_next_action(&choice("synthesize", f64::NAN), &CANDIDATES, 0.5);
        assert!(d.fallback);
    }

    #[test]
    fn a_missing_answer_falls_back_to_the_first_candidate() {
        let d = resolve_next_action(&HashMap::new(), &CANDIDATES, 0.5);
        assert_eq!(d.action, NextAction::GetEvidencePack);
        assert!(d.fallback);
    }

    #[test]
    fn a_wrongly_typed_answer_falls_back() {
        let mut m = HashMap::new();
        m.insert(NEXT_ACTION_KEY.to_string(), SystemOneAnswer::Noul { noul: 0.9 });
        let d = resolve_next_action(&m, &CANDIDATES, 0.5);
        assert!(d.fallback);
    }

    #[test]
    fn an_empty_candidate_list_falls_back_to_continue() {
        let d = resolve_next_action(&choice("synthesize", 0.99), &[], 0.5);
        assert_eq!(d.action, NextAction::Continue);
        assert!(d.fallback);
    }

    #[test]
    fn every_candidate_has_a_distinct_criterion() {
        let criteria = candidate_criteria(&CANDIDATES);
        assert_eq!(criteria.len(), CANDIDATES.len());
        let mut values: Vec<&String> = criteria.values().collect();
        values.sort();
        values.dedup();
        assert_eq!(values.len(), CANDIDATES.len(), "criteria must be distinguishable");
    }

    #[test]
    fn criteria_use_the_router_keys() {
        let criteria = candidate_criteria(&CANDIDATES);
        for a in CANDIDATES {
            assert!(criteria.contains_key(a.key()), "missing criterion for {a:?}");
        }
    }

    #[tokio::test]
    async fn routing_without_candidates_never_calls_the_provider() {
        let server = httpmock::MockServer::start();
        let client = SystemOneClient::new(
            &SystemOneConfig { endpoint: server.url("/"), api_key: "k".into(), model: "m".into(), ..Default::default() },
            "k".into(),
        );
        let d = client.route_next_action("goal", "progress", &[], 0.5).await.unwrap();
        assert_eq!(d.action, NextAction::Continue);
        assert!(d.fallback);
    }
}

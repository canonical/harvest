use std::sync::Arc;

use async_trait::async_trait;

use crate::context::SearchContext;
use crate::domain::Gap;
use crate::gate::{SufficiencyCheck, Verdict};
use crate::llm::LanguageModel;

const SYSTEM: &str = "You judge whether retrieved code evidence is enough to answer a question about a codebase. Reply with JSON only: {\"answerable\": \"yes\" | \"partly\" | \"no\", \"missing\": [short descriptions of missing code], \"confidence\": number between 0 and 1}.";
const SNIPPET_CHARS: usize = 400;
const MIN_CONFIDENCE: f64 = 0.6;

pub struct ModelJudge {
    model: Arc<dyn LanguageModel>,
}

impl ModelJudge {
    pub fn new(model: Arc<dyn LanguageModel>) -> Self {
        Self { model }
    }
}

pub fn evidence_digest(ctx: &SearchContext, snippet_chars: usize) -> String {
    ctx.evidence
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let snippet: String = e.text.chars().take(snippet_chars).collect();
            format!("[{i}] {}\n{snippet}", e.label)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[async_trait]
impl SufficiencyCheck for ModelJudge {
    fn name(&self) -> &'static str {
        "model_judge"
    }

    fn uses_model(&self) -> bool {
        true
    }

    async fn check(&self, ctx: &SearchContext) -> Verdict {
        let prompt = format!(
            "Question: {}\n\nEvidence:\n{}",
            ctx.query.text,
            evidence_digest(ctx, SNIPPET_CHARS)
        );
        let reply = match self
            .model
            .complete_json(SYSTEM, &prompt, ctx.deadline)
            .await
        {
            Ok(reply) => reply,
            Err(e) => {
                tracing::warn!(error = %e, "sufficiency judge failed");
                return Verdict::Undecided;
            }
        };
        let confidence = reply["confidence"].as_f64().unwrap_or(0.0);
        let missing: Vec<Gap> = reply["missing"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|m| m.as_str())
                    .map(|hint| Gap::Unanswered {
                        hint: hint.to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        match reply["answerable"].as_str() {
            Some("yes") if confidence >= MIN_CONFIDENCE => Verdict::Sufficient,
            Some("yes" | "partly" | "no") => Verdict::Insufficient(missing),
            _ => Verdict::Undecided,
        }
    }
}

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use harvest_search::llm::LanguageModel;
use harvest_search::{Deadline, Evidence};

const ANSWER_SYSTEM: &str = "You answer questions about a codebase using only the evidence provided. Cite code as [file:start-end]. Reply with JSON only: {\"answer\": \"...\"}.";
const JUDGE_SYSTEM: &str = "You compare two answers to a question about a codebase. Prefer the answer that is correct, complete and grounded in cited code. Reply with JSON only: {\"winner\": \"A\" | \"B\" | \"tie\", \"reason\": \"...\"}.";
const EVIDENCE_CHARS: usize = 24_000;
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

pub struct Answerer {
    model: Arc<dyn LanguageModel>,
}

impl Answerer {
    pub fn new(model: Arc<dyn LanguageModel>) -> Self {
        Self { model }
    }

    pub async fn answer(&self, question: &str, evidence: &[Evidence]) -> Result<String> {
        let mut context = String::new();
        for e in evidence {
            if context.len() >= EVIDENCE_CHARS {
                break;
            }
            context.push_str(&format!("\n--- {}\n{}\n", e.label, e.text));
        }
        let prompt = format!("Question: {question}\n\nEvidence:{context}");
        let reply = self
            .model
            .complete_json(ANSWER_SYSTEM, &prompt, Deadline::after(CALL_TIMEOUT))
            .await
            .map_err(|e| anyhow!(e.to_string()))?;
        reply["answer"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("the answer reply had no answer field"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Win,
    Tie,
    Loss,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Win => "win",
            Self::Tie => "tie",
            Self::Loss => "loss",
        }
    }
}

pub struct Judge {
    model: Arc<dyn LanguageModel>,
}

impl Judge {
    pub fn new(model: Arc<dyn LanguageModel>) -> Self {
        Self { model }
    }

    async fn pick(&self, question: &str, a: &str, b: &str) -> Result<Option<bool>> {
        let prompt = format!("Question: {question}\n\nAnswer A:\n{a}\n\nAnswer B:\n{b}");
        let reply = self
            .model
            .complete_json(JUDGE_SYSTEM, &prompt, Deadline::after(CALL_TIMEOUT))
            .await
            .map_err(|e| anyhow!(e.to_string()))?;
        Ok(match reply["winner"].as_str() {
            Some("A") => Some(true),
            Some("B") => Some(false),
            _ => None,
        })
    }

    pub async fn compare(
        &self,
        question: &str,
        candidate: &str,
        reference: &str,
    ) -> Result<Verdict> {
        let first = self.pick(question, candidate, reference).await?;
        let second = self.pick(question, reference, candidate).await?.map(|a| !a);
        Ok(match (first, second) {
            (Some(true), Some(true)) => Verdict::Win,
            (Some(false), Some(false)) => Verdict::Loss,
            _ => Verdict::Tie,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvest_search::llm::ScriptedModel;
    use serde_json::json;

    #[tokio::test]
    async fn verdicts_need_agreement_across_both_orders() {
        let judge = Judge::new(Arc::new(ScriptedModel::new(vec![
            json!({"winner": "A"}),
            json!({"winner": "B"}),
        ])));
        assert_eq!(judge.compare("q", "x", "y").await.unwrap(), Verdict::Win);
        let biased = Judge::new(Arc::new(ScriptedModel::new(vec![json!({"winner": "A"})])));
        assert_eq!(biased.compare("q", "x", "y").await.unwrap(), Verdict::Tie);
    }

    #[tokio::test]
    async fn answers_come_back_from_json() {
        let answerer = Answerer::new(Arc::new(ScriptedModel::new(vec![
            json!({"answer": "It retries."}),
        ])));
        assert_eq!(answerer.answer("q", &[]).await.unwrap(), "It retries.");
    }
}

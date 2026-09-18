use crate::llm::types::{Usage, UsedProvider};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Source {
    pub repo: String,
    pub version: String,
    pub file: String,
    pub line: u32,
    #[serde(default)]
    pub end_line: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QueryResponse {
    pub answer: String,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub tool_calls_made: usize,
    #[serde(default)]
    pub provider_used: Option<UsedProvider>,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub usage: Usage,
    #[serde(default)]
    pub llm_call_count: usize,
    #[serde(default)]
    pub cost_microusd: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_response_deserializes_full_shape() {
        let body = serde_json::json!({
            "answer": "done",
            "sources": [{"repo": "r", "version": "v1", "file": "f.rs", "line": 12}],
            "tool_calls_made": 2,
            "provider_used": {"provider_id": "anthropic-0", "kind": "anthropic", "model": "claude-sonnet-4-6"},
            "duration_ms": 500,
            "usage": {"input_tokens": 10, "output_tokens": 5, "cache_read_tokens": 0, "cache_creation_tokens": 0, "reasoning_tokens": 0},
            "llm_call_count": 1,
            "cost_microusd": 42
        });
        let resp: QueryResponse = serde_json::from_value(body).unwrap();
        assert_eq!(resp.answer, "done");
        assert_eq!(resp.sources.len(), 1);
        assert_eq!(resp.provider_used.unwrap().provider_id, "anthropic-0");
    }

    #[test]
    fn query_response_deserializes_with_only_answer_present() {
        let body = serde_json::json!({ "answer": "ok" });
        let resp: QueryResponse = serde_json::from_value(body).unwrap();
        assert_eq!(resp.answer, "ok");
        assert!(resp.sources.is_empty());
        assert!(resp.provider_used.is_none());
    }
}

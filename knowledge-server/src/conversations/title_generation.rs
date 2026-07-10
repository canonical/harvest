use crate::agent::HistoryMessage;
use crate::llm::{LlmProvider, types::Message};
use crate::neo4j::Neo4jClient;
use serde_json::json;

const TITLE_MAX_CHARS: usize = 60;
const CONTEXT_MAX_CHARS: usize = 3000;

pub fn should_regenerate_title(message_count: usize) -> bool {
    matches!(message_count, 2 | 6 | 12) || (message_count > 12 && (message_count - 12) % 10 == 0)
}

fn role_label(role: &str) -> &str {
    match role {
        "user"      => "User",
        "assistant" => "Assistant",
        "summary"   => "[Summary]",
        other       => other,
    }
}

fn build_context(prior: &[HistoryMessage], user_text: &str, assistant_text: &str) -> String {
    let n = prior.len();
    let mut parts: Vec<String> = Vec::new();

    if n <= 6 {
        for m in prior {
            parts.push(format!("{}: {}", role_label(&m.role), m.text));
        }
    } else {
        for m in &prior[..3] {
            parts.push(format!("{}: {}", role_label(&m.role), m.text));
        }
        parts.push("[...]".to_string());
        for m in &prior[n - 3..] {
            parts.push(format!("{}: {}", role_label(&m.role), m.text));
        }
    }

    parts.push(format!("User: {user_text}"));
    parts.push(format!("Assistant: {assistant_text}"));

    let full = parts.join("\n");
    full.chars().take(CONTEXT_MAX_CHARS).collect()
}

pub async fn generate_title(
    llm: &dyn LlmProvider,
    prior: &[HistoryMessage],
    user_text: &str,
    assistant_text: &str,
) -> Option<String> {
    let context = build_context(prior, user_text, assistant_text);
    let messages = vec![
        Message::system(
            "Generate a short, expressive title for this conversation. \
             Max 60 characters. No quotes. Output only the title, nothing else.",
        ),
        Message::user(context),
    ];

    let text = match llm.chat(&messages, &[]).await {
        Ok(crate::llm::types::LlmResponse::Message { text }) => text,
        _ => return None,
    };

    let title: String = text
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .chars()
        .take(TITLE_MAX_CHARS)
        .collect();

    if title.is_empty() { None } else { Some(title) }
}

async fn update_title(neo4j: &Neo4jClient, conv_id: &str, title: &str) -> bool {
    neo4j.query_read(
        "MATCH (c:Conversation {id: $cid}) SET c.title = $title RETURN c.id AS id",
        json!({ "cid": conv_id, "title": title }),
    ).await.is_ok()
}

pub async fn maybe_regenerate_title(
    neo4j: &Neo4jClient,
    llm: &dyn LlmProvider,
    conv_id: &str,
    prior: &[HistoryMessage],
    user_text: &str,
    assistant_text: &str,
    message_count: usize,
) -> Option<String> {
    if !should_regenerate_title(message_count) {
        return None;
    }
    let title = generate_title(llm, prior, user_text, assistant_text).await?;
    if update_title(neo4j, conv_id, &title).await {
        Some(title)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use crate::llm::types::{LlmResponse, ModelInfo, ToolDefinition};

    struct MockLlm(String);

    #[async_trait]
    impl LlmProvider for MockLlm {
        fn id(&self) -> &str { "mock" }
        fn kind(&self) -> &str { "mock" }
        fn default_model(&self) -> &str { "mock-model" }
        async fn list_models(&self) -> Result<Vec<ModelInfo>> { Ok(vec![]) }
        async fn chat_with(&self, _: Option<&str>, _: &[Message], _: &[ToolDefinition]) -> Result<LlmResponse> {
            Ok(LlmResponse::Message { text: self.0.clone() })
        }
    }

    struct FailLlm;

    #[async_trait]
    impl LlmProvider for FailLlm {
        fn id(&self) -> &str { "fail" }
        fn kind(&self) -> &str { "mock" }
        fn default_model(&self) -> &str { "mock-model" }
        async fn list_models(&self) -> Result<Vec<ModelInfo>> { Ok(vec![]) }
        async fn chat_with(&self, _: Option<&str>, _: &[Message], _: &[ToolDefinition]) -> Result<LlmResponse> {
            Err(anyhow::anyhow!("LLM failure"))
        }
    }

    fn msg(role: &str, text: &str) -> HistoryMessage {
        HistoryMessage { role: role.into(), text: text.into(), attachments: None }
    }

    #[test]
    fn regenerate_at_count_2() { assert!(should_regenerate_title(2)); }

    #[test]
    fn regenerate_at_count_6() { assert!(should_regenerate_title(6)); }

    #[test]
    fn regenerate_at_count_12() { assert!(should_regenerate_title(12)); }

    #[test]
    fn regenerate_at_count_22() { assert!(should_regenerate_title(22)); }

    #[test]
    fn regenerate_at_count_32() { assert!(should_regenerate_title(32)); }

    #[test]
    fn no_regenerate_at_count_0() { assert!(!should_regenerate_title(0)); }

    #[test]
    fn no_regenerate_at_count_1() { assert!(!should_regenerate_title(1)); }

    #[test]
    fn no_regenerate_at_count_3() { assert!(!should_regenerate_title(3)); }

    #[test]
    fn no_regenerate_at_count_4() { assert!(!should_regenerate_title(4)); }

    #[test]
    fn no_regenerate_at_count_5() { assert!(!should_regenerate_title(5)); }

    #[test]
    fn no_regenerate_at_count_7() { assert!(!should_regenerate_title(7)); }

    #[test]
    fn no_regenerate_at_count_11() { assert!(!should_regenerate_title(11)); }

    #[test]
    fn no_regenerate_at_count_13() { assert!(!should_regenerate_title(13)); }

    #[test]
    fn no_regenerate_at_count_21() { assert!(!should_regenerate_title(21)); }

    #[tokio::test]
    async fn generate_title_returns_llm_text() {
        let llm = MockLlm("Rust Async Patterns".into());
        let result = generate_title(&llm, &[], "How do async traits work?", "Async traits in Rust use...").await;
        assert_eq!(result, Some("Rust Async Patterns".into()));
    }

    #[tokio::test]
    async fn generate_title_strips_double_quotes() {
        let llm = MockLlm("\"Rust Async Patterns\"".into());
        let result = generate_title(&llm, &[], "question", "answer").await;
        assert_eq!(result, Some("Rust Async Patterns".into()));
    }

    #[tokio::test]
    async fn generate_title_strips_single_quotes() {
        let llm = MockLlm("'Rust Async Patterns'".into());
        let result = generate_title(&llm, &[], "question", "answer").await;
        assert_eq!(result, Some("Rust Async Patterns".into()));
    }

    #[tokio::test]
    async fn generate_title_strips_surrounding_whitespace() {
        let llm = MockLlm("  A Title With Spaces  ".into());
        let result = generate_title(&llm, &[], "q", "a").await;
        assert_eq!(result, Some("A Title With Spaces".into()));
    }

    #[tokio::test]
    async fn generate_title_truncates_to_60_chars() {
        let long = "A".repeat(100);
        let llm = MockLlm(long);
        let result = generate_title(&llm, &[], "q", "a").await;
        assert_eq!(result.map(|t| t.len()), Some(60));
    }

    #[tokio::test]
    async fn generate_title_returns_none_on_llm_failure() {
        let result = generate_title(&FailLlm, &[], "q", "a").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn generate_title_returns_none_for_empty_response() {
        let llm = MockLlm("".into());
        let result = generate_title(&llm, &[], "q", "a").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn generate_title_returns_none_for_whitespace_only() {
        let llm = MockLlm("   ".into());
        let result = generate_title(&llm, &[], "q", "a").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn generate_title_works_with_prior_history() {
        let prior = vec![
            msg("user", "What is Rust?"),
            msg("assistant", "Rust is a systems language..."),
        ];
        let llm = MockLlm("Rust Language Overview".into());
        let result = generate_title(&llm, &prior, "Tell me more", "More about Rust...").await;
        assert_eq!(result, Some("Rust Language Overview".into()));
    }

    #[test]
    fn build_context_with_no_prior_contains_current_turn() {
        let ctx = build_context(&[], "hello", "world");
        assert!(ctx.contains("User: hello"));
        assert!(ctx.contains("Assistant: world"));
    }

    #[test]
    fn build_context_with_few_prior_includes_all() {
        let prior = vec![
            msg("user", "first"),
            msg("assistant", "second"),
        ];
        let ctx = build_context(&prior, "third", "fourth");
        assert!(ctx.contains("User: first"));
        assert!(ctx.contains("Assistant: second"));
        assert!(ctx.contains("User: third"));
        assert!(ctx.contains("Assistant: fourth"));
    }

    #[test]
    fn build_context_with_many_prior_adds_ellipsis() {
        let prior: Vec<HistoryMessage> = (0..8).map(|i| msg("user", &format!("msg{i}"))).collect();
        let ctx = build_context(&prior, "current", "response");
        assert!(ctx.contains("[...]"));
        assert!(ctx.contains("msg0"));
        assert!(ctx.contains("msg7"));
    }

    #[test]
    fn build_context_caps_at_3000_chars() {
        let long_text = "x".repeat(1000);
        let prior: Vec<HistoryMessage> = (0..10).map(|_| msg("user", &long_text)).collect();
        let ctx = build_context(&prior, &long_text, &long_text);
        assert!(ctx.chars().count() <= CONTEXT_MAX_CHARS);
    }

    #[test]
    fn build_context_formats_summary_role() {
        let prior = vec![msg("summary", "key facts here")];
        let ctx = build_context(&prior, "q", "a");
        assert!(ctx.contains("[Summary]: key facts here"));
    }
}

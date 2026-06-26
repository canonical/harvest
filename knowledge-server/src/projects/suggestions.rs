use crate::llm::{LlmProvider, types::{LlmResponse, Message}};

pub async fn generate_suggestions(
    llm: &dyn LlmProvider,
    user_query: &str,
    assistant_answer: &str,
) -> Option<Vec<String>> {
    let messages = vec![Message::user(format!(
        "Based on this exchange, suggest exactly 3 concise follow-up questions or actions \
         the user might want next. Each must be under 10 words. \
         Do not include generic options like \"Other\" or \"Something else\". \
         Reply ONLY with a JSON array of 3 strings, no prose.\n\n\
         User: {user_query}\nAssistant: {assistant_answer}",
    ))];

    let text = match llm.chat(&messages, &[]).await {
        Ok(LlmResponse::Message { text }) => text,
        Err(e) => {
            tracing::warn!(error=%e, "suggestions LLM call failed");
            return None;
        }
        _ => return None,
    };

    tracing::debug!(raw=%text, "suggestions LLM raw response");
    let result = parse_json_array(&text);
    tracing::info!(ok=%result.is_some(), "suggestions generated");
    result
}

fn parse_json_array(text: &str) -> Option<Vec<String>> {
    let trimmed = text.trim();
    let start = trimmed.find('[')?;
    let end   = trimmed.rfind(']')?;
    let choices: Vec<String> = serde_json::from_str(&trimmed[start..=end]).ok()?;
    let filtered: Vec<String> = choices.into_iter().filter(|s| !is_catchall(s)).collect();
    if filtered.is_empty() { None } else { Some(filtered) }
}

fn is_catchall(s: &str) -> bool {
    let lower = s.trim().to_lowercase();
    let stripped = lower.trim_end_matches(|c: char| matches!(c, '.' | '?' | '!') || c == '\u{2026}');
    matches!(stripped.trim(), "other" | "something else" | "none of the above" | "other option")
}

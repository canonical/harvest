use crate::llm::types::{ContentPart, Message, MessageContent, ToolDefinition};

const CHARS_PER_TOKEN: usize = 4;
const MESSAGE_OVERHEAD_TOKENS: usize = 4;
const IMAGE_TOKENS: usize = 1_600;
const DOCUMENT_BYTES_PER_TOKEN: usize = 3;
const RESERVED_OUTPUT_TOKENS: usize = 8_192;

pub fn text_tokens(text: &str) -> usize {
    text.len().div_ceil(CHARS_PER_TOKEN)
}

pub fn message_tokens(message: &Message) -> usize {
    let content = match &message.content {
        MessageContent::Text(text) => text_tokens(text),
        MessageContent::Parts(parts) => parts.iter().map(part_tokens).sum(),
    };
    content + MESSAGE_OVERHEAD_TOKENS
}

fn part_tokens(part: &ContentPart) -> usize {
    match part {
        ContentPart::Text { text, .. } => text_tokens(text),
        ContentPart::ToolUse { name, input, .. } => {
            text_tokens(name) + text_tokens(&input.to_string())
        }
        ContentPart::ToolResult { content, .. } => text_tokens(content),
        ContentPart::Image { .. } => IMAGE_TOKENS,
        ContentPart::Document { data, .. } => data.len() / DOCUMENT_BYTES_PER_TOKEN,
    }
}

pub fn tools_tokens(tools: &[ToolDefinition]) -> usize {
    tools
        .iter()
        .map(|t| {
            text_tokens(&t.name)
                + text_tokens(&t.description)
                + text_tokens(&t.parameters.to_string())
        })
        .sum()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextBudget {
    window: usize,
}

#[derive(Debug)]
pub struct Fitted {
    pub messages: Vec<Message>,
    pub dropped: usize,
    pub tokens: usize,
}

impl ContextBudget {
    pub fn for_window(window: Option<usize>) -> Option<Self> {
        window
            .filter(|w| *w > RESERVED_OUTPUT_TOKENS)
            .map(|window| Self { window })
    }

    fn available(&self, fixed_tokens: usize) -> usize {
        self.window
            .saturating_sub(RESERVED_OUTPUT_TOKENS)
            .saturating_sub(fixed_tokens)
    }

    pub fn fit(&self, fixed_tokens: usize, pinned: Vec<Message>, history: Vec<Message>) -> Fitted {
        let pinned_tokens: usize = pinned.iter().map(message_tokens).sum();
        let mut remaining = self.available(fixed_tokens).saturating_sub(pinned_tokens);
        let mut kept: Vec<Message> = Vec::with_capacity(history.len());
        let mut dropped = 0;
        for message in history.into_iter().rev() {
            let cost = message_tokens(&message);
            if dropped == 0 && cost <= remaining {
                remaining -= cost;
                kept.push(message);
            } else {
                dropped += 1;
            }
        }
        kept.reverse();
        let used: usize = kept.iter().map(message_tokens).sum::<usize>() + pinned_tokens;
        let mut messages = pinned;
        if dropped > 0 {
            messages.push(Message::user(format!(
                "[{dropped} earlier messages were left out to fit the context window]"
            )));
        }
        messages.extend(kept);
        Fitted {
            messages,
            dropped,
            tokens: used + fixed_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(len: usize) -> Message {
        Message::user("x".repeat(len))
    }

    #[test]
    fn windows_too_small_for_the_reserved_output_disable_budgeting() {
        assert_eq!(ContextBudget::for_window(None), None);
        assert_eq!(ContextBudget::for_window(Some(4_000)), None);
        assert!(ContextBudget::for_window(Some(32_000)).is_some());
    }

    #[test]
    fn everything_fits_in_a_large_window() {
        let budget = ContextBudget::for_window(Some(200_000)).unwrap();
        let fitted = budget.fit(1_000, vec![], vec![text(400), text(400)]);
        assert_eq!(fitted.dropped, 0);
        assert_eq!(fitted.messages.len(), 2);
    }

    #[test]
    fn the_oldest_messages_are_dropped_first_and_the_gap_is_announced() {
        let budget = ContextBudget::for_window(Some(RESERVED_OUTPUT_TOKENS + 1_000)).unwrap();
        let history = vec![
            Message::user("oldest ".repeat(800)),
            Message::user("older ".repeat(800)),
            Message::user("recent"),
        ];
        let fitted = budget.fit(0, vec![], history);
        assert_eq!(fitted.dropped, 2);
        assert!(
            matches!(&fitted.messages[0].content, MessageContent::Text(t) if t.contains("2 earlier messages"))
        );
        assert!(matches!(&fitted.messages[1].content, MessageContent::Text(t) if t == "recent"));
        assert!(fitted.tokens <= 1_000);
    }

    #[test]
    fn pinned_messages_always_survive() {
        let budget = ContextBudget::for_window(Some(RESERVED_OUTPUT_TOKENS + 100)).unwrap();
        let pinned = vec![Message::user("summary of everything")];
        let fitted = budget.fit(0, pinned, vec![text(10_000)]);
        assert_eq!(fitted.dropped, 1);
        assert!(
            matches!(&fitted.messages[0].content, MessageContent::Text(t) if t == "summary of everything")
        );
    }

    #[test]
    fn a_gap_is_never_bridged_by_an_older_small_message() {
        let budget = ContextBudget::for_window(Some(RESERVED_OUTPUT_TOKENS + 300)).unwrap();
        let fitted = budget.fit(0, vec![], vec![text(8), text(4_000), text(8)]);
        assert_eq!(fitted.dropped, 2);
        assert_eq!(fitted.messages.len(), 2);
    }

    #[test]
    fn images_are_costed_as_a_fixed_block() {
        let message = Message {
            role: crate::llm::types::Role::User,
            content: MessageContent::Parts(vec![ContentPart::Image {
                media_type: "image/png".into(),
                data: "a".repeat(1_000_000),
            }]),
        };
        assert_eq!(
            message_tokens(&message),
            IMAGE_TOKENS + MESSAGE_OVERHEAD_TOKENS
        );
    }
}

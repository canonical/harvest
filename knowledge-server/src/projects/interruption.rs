use anyhow::Result;
use harvest_db::Db;
use serde_json::{json, Value};

use crate::conversations::store::{self, Guard, Owner};
use crate::projects::live::{InFlightEvent, InFlightState};

pub const STATUS: &str = "interrupted";

pub struct Interruption<'a> {
    pub project_id: &'a str,
    pub conv_id: &'a str,
    pub turn_id: &'a str,
    pub username: &'a str,
    pub query: &'a str,
    pub attachments: &'a [Value],
    pub partial_answer: String,
    pub chain: Vec<Value>,
}

impl Interruption<'_> {
    fn messages(&self) -> [Value; 2] {
        [
            json!({
                "role": "user",
                "text": self.query,
                "username": self.username,
                "attachments": self.attachments,
            }),
            json!({
                "role": "assistant",
                "text": self.partial_answer,
                "status": STATUS,
                "sources": [],
                "chain": self.chain,
            }),
        ]
    }

    pub async fn record(&self, db: &Db) -> Result<bool> {
        store::append(
            db,
            Owner::Project(self.project_id),
            Guard::HoldingTurn(self.turn_id),
            self.conv_id,
            &self.messages(),
            &harvest_db::now_rfc3339(),
        )
        .await
    }
}

pub fn partial_answer(state: &InFlightState) -> String {
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    for event in &state.events {
        match event {
            InFlightEvent::TextDelta { text } => current.push_str(text),
            _ if !current.trim().is_empty() => segments.push(std::mem::take(&mut current)),
            _ => current.clear(),
        }
    }
    if !current.trim().is_empty() {
        segments.push(current);
    }
    segments
        .iter()
        .map(|s| s.trim())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn chain_of(state: &InFlightState) -> Vec<Value> {
    let mut chain = crate::agent::chain::ChainBuilder::new();
    for event in &state.events {
        match event {
            InFlightEvent::Thinking { text } => chain.thinking(text),
            InFlightEvent::TextDelta { .. } => {}
            InFlightEvent::ToolCall {
                name,
                input,
                description,
                hostname,
            } => chain.tool_call(name, input, description.as_deref(), hostname.as_deref()),
            InFlightEvent::ToolResult { name, preview } => chain.tool_result(name, preview),
        }
    }
    chain.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_partial_answer_joins_streamed_text_only() {
        let state = InFlightState {
            events: vec![
                InFlightEvent::Thinking { text: "hmm".into() },
                InFlightEvent::TextDelta {
                    text: "Hello ".into(),
                },
                InFlightEvent::ToolResult {
                    name: "t".into(),
                    preview: "p".into(),
                },
                InFlightEvent::TextDelta {
                    text: "world".into(),
                },
            ],
            ..InFlightState::default()
        };
        assert_eq!(partial_answer(&state), "Hello\n\nworld");
        let chain = chain_of(&state);
        assert_eq!(chain[0]["type"], "thinking");
    }

    #[test]
    fn interrupted_answers_are_marked() {
        let interruption = Interruption {
            project_id: "p",
            conv_id: "c",
            turn_id: "t",
            username: "ann",
            query: "q",
            attachments: &[],
            partial_answer: "half".into(),
            chain: vec![serde_json::json!({"type": "tool_call", "name": "search_code"})],
        };
        let [user, assistant] = interruption.messages();
        assert_eq!(user["text"], "q");
        assert_eq!(assistant["status"], STATUS);
        assert_eq!(assistant["text"], "half");
    }
}

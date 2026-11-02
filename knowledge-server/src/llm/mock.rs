use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::config::MockBehaviour;
use crate::llm::types::{
    LlmResponse, Message, ModelInfo, ProviderMeta, StreamEvent, ToolDefinition, Usage,
};
use crate::llm::LlmProvider;

pub struct MockProvider {
    model: String,
    behaviour: MockBehaviour,
    meta: ProviderMeta,
}

impl MockProvider {
    pub fn new(model: String, behaviour: MockBehaviour, meta: ProviderMeta) -> Self {
        Self {
            model,
            behaviour,
            meta,
        }
    }

    fn chunks(&self) -> Vec<String> {
        match &self.behaviour.reply {
            Some(reply) => split_into(reply, self.behaviour.chunks.max(1)),
            None => (0..self.behaviour.chunks)
                .map(|i| format!("chunk{i} "))
                .collect(),
        }
    }

    fn usage(&self, chunks: usize) -> Usage {
        Usage {
            input_tokens: 1,
            output_tokens: chunks as u64,
            ..Usage::default()
        }
    }
}

fn split_into(text: &str, parts: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let size = chars.len().div_ceil(parts).max(1);
    chars
        .chunks(size)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn id(&self) -> &str {
        &self.meta.id
    }

    fn kind(&self) -> &str {
        "mock"
    }

    fn default_model(&self) -> &str {
        &self.model
    }

    fn name(&self) -> Option<&str> {
        self.meta.name.as_deref()
    }

    fn expose_to_ui(&self) -> bool {
        self.meta.expose_to_ui
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: self.model.clone(),
            display_name: None,
        }])
    }

    async fn chat_with(
        &self,
        _model: Option<&str>,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        tokio::time::sleep(Duration::from_millis(self.behaviour.first_token_ms)).await;
        let chunks = self.chunks();
        Ok(LlmResponse::Message {
            usage: self.usage(chunks.len()),
            text: chunks.concat(),
        })
    }

    async fn chat_stream_with(
        &self,
        _model: Option<&str>,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        tokio::time::sleep(Duration::from_millis(self.behaviour.first_token_ms)).await;
        let chunks = self.chunks();
        let count = chunks.len();
        for (index, text) in chunks.into_iter().enumerate() {
            if index > 0 {
                tokio::time::sleep(Duration::from_millis(self.behaviour.chunk_delay_ms)).await;
            }
            if tx.send(StreamEvent::TextDelta { text }).await.is_err() {
                return Ok(());
            }
        }
        let _ = tx
            .send(StreamEvent::Done {
                stop_reason: "end_turn".into(),
                usage: self.usage(count),
            })
            .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(reply: Option<&str>, chunks: usize) -> MockProvider {
        MockProvider::new(
            "mock".into(),
            MockBehaviour {
                chunks,
                chunk_delay_ms: 0,
                first_token_ms: 0,
                reply: reply.map(str::to_string),
            },
            ProviderMeta {
                id: "mock".into(),
                expose_to_ui: true,
                name: None,
                models: None,
                user_provided_key: false,
            },
        )
    }

    #[tokio::test]
    async fn streams_the_configured_number_of_chunks_then_done() {
        let (tx, mut rx) = mpsc::channel(16);
        provider(None, 3)
            .chat_stream_with(None, &[], &[], tx)
            .await
            .unwrap();
        let mut texts = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::TextDelta { text } => texts.push(text),
                StreamEvent::Done { .. } => break,
                _ => {}
            }
        }
        assert_eq!(texts, vec!["chunk0 ", "chunk1 ", "chunk2 "]);
    }

    #[tokio::test]
    async fn a_configured_reply_is_split_across_chunks() {
        let response = provider(Some("hello world"), 4)
            .chat_with(None, &[], &[])
            .await
            .unwrap();
        assert!(matches!(response, LlmResponse::Message { text, .. } if text == "hello world"));
        assert_eq!(split_into("hello world", 4).len(), 4);
    }
}

use std::sync::Arc;

use async_trait::async_trait;
use harvest_search::llm::{parse_json_object, LanguageModel};
use harvest_search::{Deadline, SearchError};
use serde_json::Value;

use crate::llm::types::{LlmResponse, Message, ProviderSelection};
use crate::llm::LlmProvider;

pub struct ProviderModel {
    llm: Arc<dyn LlmProvider>,
    selection: Option<ProviderSelection>,
}

impl ProviderModel {
    pub fn new(llm: Arc<dyn LlmProvider>, selection: Option<ProviderSelection>) -> Arc<Self> {
        Arc::new(Self { llm, selection })
    }
}

#[async_trait]
impl LanguageModel for ProviderModel {
    async fn complete_json(
        &self,
        system: &str,
        prompt: &str,
        deadline: Deadline,
    ) -> Result<Value, SearchError> {
        let messages = [Message::system(system), Message::user(prompt)];
        let call = self
            .llm
            .chat_routed(self.selection.as_ref(), &messages, &[]);
        let (response, _, _) = tokio::time::timeout(deadline.remaining(), call)
            .await
            .map_err(|_| SearchError::Model("the model did not answer before the deadline".into()))?
            .map_err(|e| SearchError::Model(e.to_string()))?;
        match response {
            LlmResponse::Message { text, .. } => parse_json_object(&text),
            LlmResponse::ToolCalls { preamble, .. } => parse_json_object(&preamble),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MockBehaviour;
    use crate::llm::mock::MockProvider;
    use crate::llm::types::ProviderMeta;
    use std::time::Duration;

    fn mock(reply: &str, first_token_ms: u64) -> Arc<dyn LlmProvider> {
        Arc::new(MockProvider::new(
            "m".into(),
            MockBehaviour {
                chunks: 1,
                chunk_delay_ms: 0,
                first_token_ms,
                reply: Some(reply.into()),
            },
            ProviderMeta::new("mock"),
        ))
    }

    #[tokio::test]
    async fn slow_models_fail_at_the_deadline() {
        let model = ProviderModel::new(mock("{}", 500), None);
        let reply = model
            .complete_json("s", "p", Deadline::after(Duration::from_millis(20)))
            .await;
        assert!(matches!(reply, Err(SearchError::Model(_))));
    }

    #[tokio::test]
    async fn replies_are_parsed_as_json() {
        let model = ProviderModel::new(mock("```json\n{\"steps\": []}\n```", 0), None);
        let reply = model
            .complete_json("s", "p", Deadline::after(Duration::from_secs(5)))
            .await
            .unwrap();
        assert!(reply["steps"].is_array());
    }
}

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::Value;

use crate::deadline::Deadline;
use crate::domain::SearchError;

#[async_trait]
pub trait LanguageModel: Send + Sync {
    async fn complete_json(
        &self,
        system: &str,
        prompt: &str,
        deadline: Deadline,
    ) -> Result<Value, SearchError>;
}

pub fn parse_json_object(text: &str) -> Result<Value, SearchError> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if value.is_object() {
            return Ok(value);
        }
    }
    let start = trimmed.find('{');
    let end = trimmed.rfind('}');
    match (start, end) {
        (Some(start), Some(end)) if end > start => serde_json::from_str(&trimmed[start..=end])
            .map_err(|e| SearchError::Model(format!("the model did not return valid JSON: {e}"))),
        _ => Err(SearchError::Model(
            "the model did not return a JSON object".into(),
        )),
    }
}

pub struct ScriptedModel {
    replies: Mutex<VecDeque<Result<Value, String>>>,
    prompts: Mutex<Vec<String>>,
}

impl ScriptedModel {
    pub fn new(replies: Vec<Value>) -> Self {
        Self {
            replies: Mutex::new(replies.into_iter().map(Ok).collect()),
            prompts: Mutex::new(Vec::new()),
        }
    }

    pub fn failing() -> Self {
        Self {
            replies: Mutex::new(VecDeque::from([Err("unavailable".to_string())])),
            prompts: Mutex::new(Vec::new()),
        }
    }

    pub fn prompts(&self) -> Vec<String> {
        self.prompts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[async_trait]
impl LanguageModel for ScriptedModel {
    async fn complete_json(
        &self,
        _system: &str,
        prompt: &str,
        _deadline: Deadline,
    ) -> Result<Value, SearchError> {
        self.prompts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(prompt.to_string());
        let mut replies = self.replies.lock().unwrap_or_else(|e| e.into_inner());
        let reply = if replies.len() > 1 {
            replies.pop_front()
        } else {
            replies.front().cloned()
        };
        match reply {
            Some(Ok(value)) => Ok(value),
            Some(Err(message)) => Err(SearchError::Model(message)),
            None => Err(SearchError::Model("no scripted reply".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_is_found_inside_prose_and_fences() {
        assert_eq!(parse_json_object("{\"a\":1}").unwrap(), json!({"a": 1}));
        assert_eq!(
            parse_json_object("Sure:\n```json\n{\"steps\": []}\n```").unwrap(),
            json!({"steps": []})
        );
        assert!(parse_json_object("no json here").is_err());
        assert!(parse_json_object("[1,2]").is_err());
    }

    #[tokio::test]
    async fn scripted_replies_repeat_the_last_one() {
        let model = ScriptedModel::new(vec![json!({"n": 1}), json!({"n": 2})]);
        let deadline = Deadline::after(std::time::Duration::from_secs(1));
        assert_eq!(
            model.complete_json("", "a", deadline).await.unwrap()["n"],
            1
        );
        assert_eq!(
            model.complete_json("", "b", deadline).await.unwrap()["n"],
            2
        );
        assert_eq!(
            model.complete_json("", "c", deadline).await.unwrap()["n"],
            2
        );
        assert_eq!(model.prompts(), vec!["a", "b", "c"]);
    }
}

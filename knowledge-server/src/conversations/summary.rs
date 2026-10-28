use std::sync::Arc;

use harvest_db::Db;
use serde_json::{json, Value};

use crate::agent::Agent;
use crate::cluster::singleton;
use crate::conversations::store::{load_context, ConversationContext, Owner};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StoredSummary {
    pub text: String,
    pub upto: usize,
}

impl StoredSummary {
    pub fn from_row(row: &Value) -> Option<Self> {
        let text = row.get("summary")?.as_str()?;
        let upto = row.get("summary_upto")?.as_u64()? as usize;
        (!text.is_empty() && upto > 0).then(|| Self {
            text: text.to_string(),
            upto,
        })
    }
}

pub async fn next_summary(agent: &Agent, context: &ConversationContext) -> Option<StoredSummary> {
    let recent = context.recent_history();
    if recent.len() <= agent.compaction_keep_last() {
        return None;
    }
    let (text, kept) = agent.summarize_history(&context.history()).await?;
    Some(StoredSummary {
        text,
        upto: context.message_count - kept,
    })
}

pub async fn store(db: &Db, conv_id: &str, summary: &StoredSummary) -> bool {
    db.query(
        "UPDATE conversations
         SET summary = $summary, summary_upto = $upto
         WHERE id = $cid AND summary_upto < $upto
         RETURNING id",
        json!({ "cid": conv_id, "summary": summary.text, "upto": summary.upto as i64 }),
    )
    .await
    .map(|rows| !rows.is_empty())
    .unwrap_or(false)
}

pub async fn refresh(db: &Db, agent: &Agent, conv_id: &str) {
    let key = format!("summary:{conv_id}");
    let ran = singleton::run_exclusive(db, &key, || summarize(db, agent, conv_id)).await;
    match ran {
        Ok(true) => {}
        Ok(false) => tracing::debug!(
            conv_id,
            "another node is already summarizing this conversation"
        ),
        Err(e) => tracing::warn!(error = %e, conv_id, "could not coordinate the summary refresh"),
    }
}

async fn summarize(db: &Db, agent: &Agent, conv_id: &str) {
    let context = match load_context(db, Owner::Unscoped, conv_id).await {
        Ok(context) => context,
        Err(e) => {
            tracing::warn!(error = %e, conv_id, "could not load conversation for summary refresh");
            return;
        }
    };
    if let Some(next) = next_summary(agent, &context).await {
        if store(db, conv_id, &next).await {
            tracing::info!(conv_id, upto = next.upto, "stored conversation summary");
        }
    }
}

pub fn spawn_refresh(db: Arc<Db>, agent: Arc<Agent>, conv_id: String) {
    tokio::spawn(async move { refresh(&db, &agent, &conv_id).await });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::HistoryMessage;
    use crate::llm::types::{
        LlmResponse, Message, MessageContent, ModelInfo, ToolDefinition, Usage,
    };
    use crate::llm::LlmProvider;
    use anyhow::Result;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct RecordingLlm {
        reply: Option<String>,
        prompts: Mutex<Vec<String>>,
        delay: std::time::Duration,
    }

    impl RecordingLlm {
        fn new(reply: Option<&str>) -> Arc<Self> {
            Self::slow(reply, std::time::Duration::ZERO)
        }

        fn slow(reply: Option<&str>, delay: std::time::Duration) -> Arc<Self> {
            Arc::new(Self {
                reply: reply.map(str::to_string),
                prompts: Mutex::new(vec![]),
                delay,
            })
        }

        fn prompts(&self) -> Vec<String> {
            self.prompts.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LlmProvider for RecordingLlm {
        fn id(&self) -> &str {
            "recording"
        }
        fn kind(&self) -> &str {
            "mock"
        }
        fn default_model(&self) -> &str {
            "mock-model"
        }
        async fn list_models(&self) -> Result<Vec<ModelInfo>> {
            Ok(vec![])
        }
        async fn chat_with(
            &self,
            _: Option<&str>,
            messages: &[Message],
            _: &[ToolDefinition],
        ) -> Result<LlmResponse> {
            let prompt = messages
                .iter()
                .map(|m| match &m.content {
                    MessageContent::Text(t) => t.clone(),
                    MessageContent::Parts(_) => String::new(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            self.prompts.lock().unwrap().push(prompt);
            tokio::time::sleep(self.delay).await;
            match &self.reply {
                Some(text) => Ok(LlmResponse::Message {
                    text: text.clone(),
                    usage: Usage::default(),
                }),
                None => Err(anyhow::anyhow!("LLM failure")),
            }
        }
    }

    fn msg(role: &str, text: &str) -> HistoryMessage {
        HistoryMessage {
            role: role.into(),
            text: text.into(),
            ..Default::default()
        }
    }

    fn texts(history: &[HistoryMessage]) -> Vec<(String, String)> {
        history
            .iter()
            .map(|m| (m.role.clone(), m.text.clone()))
            .collect()
    }

    fn conversation(turns: usize) -> Vec<HistoryMessage> {
        (0..turns)
            .flat_map(|i| {
                [
                    msg("user", &format!("question {i}")),
                    msg("assistant", &format!("answer {i}")),
                ]
            })
            .collect()
    }

    fn context(turns: usize, previous: Option<&StoredSummary>) -> ConversationContext {
        let history = conversation(turns);
        let upto = previous.map_or(0, |p| p.upto);
        ConversationContext {
            summary: previous.cloned(),
            recent: history[upto..]
                .iter()
                .map(|m| json!({ "role": m.role, "text": m.text }))
                .collect(),
            message_count: history.len(),
        }
    }

    fn agent(llm: Arc<RecordingLlm>, threshold: usize, keep_last: usize) -> Agent {
        Agent::new(llm, vec![], 5).with_compaction(threshold, keep_last)
    }

    #[test]
    fn from_row_requires_text_and_a_positive_upto() {
        assert_eq!(
            StoredSummary::from_row(&json!({ "summary": "s", "summary_upto": 4 })),
            Some(StoredSummary {
                text: "s".into(),
                upto: 4
            }),
        );
        assert_eq!(
            StoredSummary::from_row(&json!({ "summary": null, "summary_upto": 0 })),
            None
        );
        assert_eq!(
            StoredSummary::from_row(&json!({ "summary": "s", "summary_upto": 0 })),
            None
        );
    }

    #[tokio::test]
    async fn next_summary_skips_history_under_the_threshold() {
        let llm = RecordingLlm::new(Some("unused"));
        let agent = agent(llm.clone(), 10_000, 2);
        assert_eq!(next_summary(&agent, &context(5, None)).await, None);
        assert!(llm.prompts().is_empty());
    }

    #[tokio::test]
    async fn next_summary_covers_all_but_the_kept_messages() {
        let llm = RecordingLlm::new(Some("summary v1"));
        let agent = agent(llm.clone(), 10, 2);
        let next = next_summary(&agent, &context(5, None)).await;
        assert_eq!(
            next,
            Some(StoredSummary {
                text: "summary v1".into(),
                upto: 8
            })
        );
    }

    #[tokio::test]
    async fn next_summary_folds_only_new_messages_into_the_previous_summary() {
        let llm = RecordingLlm::new(Some("summary v2"));
        let agent = agent(llm.clone(), 10, 2);
        let previous = StoredSummary {
            text: "summary v1".into(),
            upto: 6,
        };
        let next = next_summary(&agent, &context(6, Some(&previous))).await;
        assert_eq!(
            next,
            Some(StoredSummary {
                text: "summary v2".into(),
                upto: 10
            })
        );
        let prompt = &llm.prompts()[0];
        assert!(prompt.contains("summary v1"));
        assert!(prompt.contains("question 3") && prompt.contains("answer 4"));
        assert!(
            !prompt.contains("question 2"),
            "messages already summarized must not be re-sent"
        );
        assert!(
            !prompt.contains("question 5"),
            "kept messages must not be summarized"
        );
    }

    #[tokio::test]
    async fn next_summary_waits_until_more_than_keep_last_new_messages_exist() {
        let llm = RecordingLlm::new(Some("unused"));
        let agent = agent(llm.clone(), 10, 2);
        let previous = StoredSummary {
            text: "x".repeat(100),
            upto: 8,
        };
        assert_eq!(
            next_summary(&agent, &context(5, Some(&previous))).await,
            None
        );
        assert!(llm.prompts().is_empty());
    }

    #[tokio::test]
    async fn next_summary_returns_none_when_the_llm_fails() {
        let agent = agent(RecordingLlm::new(None), 10, 2);
        assert_eq!(next_summary(&agent, &context(5, None)).await, None);
    }

    async fn insert_conversation(db: &Db, messages: &[HistoryMessage]) -> (String, String) {
        let uid = uuid::Uuid::new_v4().to_string();
        let cid = uuid::Uuid::new_v4().to_string();
        db.query(
            "INSERT INTO users (id, provider) VALUES ($uid, 'local') RETURNING id",
            json!({ "uid": uid }),
        )
        .await
        .unwrap();
        let raw: Vec<Value> = messages
            .iter()
            .map(|m| json!({ "role": m.role, "text": m.text }))
            .collect();
        let now = harvest_db::now_rfc3339();
        crate::conversations::store::ensure_user_conversation(db, &uid, &cid, "t", &now)
            .await
            .unwrap();
        crate::conversations::store::append(
            db,
            Owner::User(&uid),
            crate::conversations::store::Guard::Unguarded,
            &cid,
            &raw,
            &now,
        )
        .await
        .unwrap();
        (uid, cid)
    }

    async fn stored(db: &Db, cid: &str) -> Option<StoredSummary> {
        let rows = db
            .query(
                "SELECT summary, summary_upto FROM conversations WHERE id = $cid",
                json!({ "cid": cid }),
            )
            .await
            .unwrap();
        StoredSummary::from_row(&rows[0])
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn refresh_stores_a_summary_that_the_next_load_uses() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let (uid, cid) = insert_conversation(&test_db.db, &conversation(5)).await;
        let agent = agent(RecordingLlm::new(Some("summary v1")), 10, 2);

        refresh(&test_db.db, &agent, &cid).await;

        assert_eq!(
            stored(&test_db.db, &cid).await,
            Some(StoredSummary {
                text: "summary v1".into(),
                upto: 8
            })
        );
        let loaded = load_context(&test_db.db, Owner::User(&uid), &cid)
            .await
            .unwrap();
        assert_eq!(loaded.message_count, 10);
        assert_eq!(
            texts(&loaded.history()),
            vec![
                ("summary".to_string(), "summary v1".to_string()),
                ("user".to_string(), "question 4".to_string()),
                ("assistant".to_string(), "answer 4".to_string()),
            ]
        );
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn store_never_replaces_a_summary_with_one_covering_fewer_messages() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let (_, cid) = insert_conversation(&test_db.db, &conversation(5)).await;
        let newer = StoredSummary {
            text: "newer".into(),
            upto: 8,
        };
        let older = StoredSummary {
            text: "older".into(),
            upto: 4,
        };

        assert!(store(&test_db.db, &cid, &newer).await);
        assert!(!store(&test_db.db, &cid, &older).await);
        assert_eq!(stored(&test_db.db, &cid).await, Some(newer));
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn concurrent_refreshes_summarize_once() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let (_, cid) = insert_conversation(&test_db.db, &conversation(5)).await;
        let llm = RecordingLlm::slow(Some("summary v1"), std::time::Duration::from_millis(300));
        let agent = agent(llm.clone(), 10, 2);
        tokio::join!(
            refresh(&test_db.db, &agent, &cid),
            refresh(&test_db.db, &agent, &cid)
        );
        assert_eq!(llm.prompts().len(), 1);
    }
}

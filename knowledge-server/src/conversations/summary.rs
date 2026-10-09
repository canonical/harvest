use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

use harvest_db::Db;
use serde_json::{json, Value};

use crate::agent::{Agent, HistoryMessage};
use crate::conversations::handlers::history_messages_from_raw;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StoredSummary {
    pub text: String,
    pub upto: usize,
}

impl StoredSummary {
    pub fn from_row(row: &Value) -> Option<Self> {
        let text = row.get("summary")?.as_str()?;
        let upto = row.get("summary_upto")?.as_u64()? as usize;
        (!text.is_empty() && upto > 0).then(|| Self { text: text.to_string(), upto })
    }
}

pub fn effective_history(history: &[HistoryMessage], summary: Option<&StoredSummary>) -> Vec<HistoryMessage> {
    let Some(summary) = summary.filter(|s| s.upto <= history.len()) else {
        return history.to_vec();
    };
    let mut out = Vec::with_capacity(1 + history.len() - summary.upto);
    out.push(HistoryMessage { role: "summary".into(), text: summary.text.clone(), ..Default::default() });
    out.extend_from_slice(&history[summary.upto..]);
    out
}

pub async fn next_summary(
    agent: &Agent,
    history: &[HistoryMessage],
    previous: Option<&StoredSummary>,
) -> Option<StoredSummary> {
    let start = previous.filter(|s| s.upto <= history.len()).map_or(0, |s| s.upto);
    if history.len() - start <= agent.compaction_keep_last() {
        return None;
    }
    let effective = effective_history(history, previous);
    let (text, kept) = agent.summarize_history(&effective).await?;
    Some(StoredSummary { text, upto: history.len() - kept })
}

pub async fn store(db: &Db, conv_id: &str, summary: &StoredSummary) -> bool {
    db.query(
        "UPDATE conversations
         SET summary = $summary, summary_upto = $upto
         WHERE id = $cid AND summary_upto < $upto
         RETURNING id",
        json!({ "cid": conv_id, "summary": summary.text, "upto": summary.upto as i64 }),
    ).await
    .map(|rows| !rows.is_empty())
    .unwrap_or(false)
}

fn refreshing() -> &'static Mutex<HashSet<String>> {
    static REFRESHING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    REFRESHING.get_or_init(Default::default)
}

struct RefreshGuard(String);

impl RefreshGuard {
    fn acquire(conv_id: &str) -> Option<Self> {
        let mut set = refreshing().lock().unwrap_or_else(|e| e.into_inner());
        set.insert(conv_id.to_string()).then(|| Self(conv_id.to_string()))
    }
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        refreshing().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

pub async fn refresh(db: &Db, agent: &Agent, conv_id: &str) {
    let Some(_guard) = RefreshGuard::acquire(conv_id) else { return };
    let rows = match db.query(
        "SELECT messages, summary, summary_upto FROM conversations WHERE id = $cid",
        json!({ "cid": conv_id }),
    ).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, conv_id, "could not load conversation for summary refresh");
            return;
        }
    };
    let Some(row) = rows.into_iter().next() else { return };
    let raw: Vec<Value> = row.get("messages")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let history = history_messages_from_raw(&raw);
    let previous = StoredSummary::from_row(&row);
    if let Some(next) = next_summary(agent, &history, previous.as_ref()).await {
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
    use anyhow::Result;
    use async_trait::async_trait;
    use crate::llm::types::{LlmResponse, Message, MessageContent, ModelInfo, ToolDefinition, Usage};
    use crate::llm::LlmProvider;

    struct RecordingLlm {
        reply: Option<String>,
        prompts: Mutex<Vec<String>>,
    }

    impl RecordingLlm {
        fn new(reply: Option<&str>) -> Arc<Self> {
            Arc::new(Self { reply: reply.map(str::to_string), prompts: Mutex::new(vec![]) })
        }

        fn prompts(&self) -> Vec<String> {
            self.prompts.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LlmProvider for RecordingLlm {
        fn id(&self) -> &str { "recording" }
        fn kind(&self) -> &str { "mock" }
        fn default_model(&self) -> &str { "mock-model" }
        async fn list_models(&self) -> Result<Vec<ModelInfo>> { Ok(vec![]) }
        async fn chat_with(&self, _: Option<&str>, messages: &[Message], _: &[ToolDefinition]) -> Result<LlmResponse> {
            let prompt = messages.iter().map(|m| match &m.content {
                MessageContent::Text(t) => t.clone(),
                MessageContent::Parts(_) => String::new(),
            }).collect::<Vec<_>>().join("\n");
            self.prompts.lock().unwrap().push(prompt);
            match &self.reply {
                Some(text) => Ok(LlmResponse::Message { text: text.clone(), usage: Usage::default() }),
                None => Err(anyhow::anyhow!("LLM failure")),
            }
        }
    }

    fn msg(role: &str, text: &str) -> HistoryMessage {
        HistoryMessage { role: role.into(), text: text.into(), ..Default::default() }
    }

    fn texts(history: &[HistoryMessage]) -> Vec<(String, String)> {
        history.iter().map(|m| (m.role.clone(), m.text.clone())).collect()
    }

    fn conversation(turns: usize) -> Vec<HistoryMessage> {
        (0..turns).flat_map(|i| [msg("user", &format!("question {i}")), msg("assistant", &format!("answer {i}"))]).collect()
    }

    fn agent(llm: Arc<RecordingLlm>, threshold: usize, keep_last: usize) -> Agent {
        Agent::new(llm, vec![], 5).with_compaction(threshold, keep_last)
    }

    #[test]
    fn effective_history_without_summary_is_the_full_history() {
        let history = conversation(3);
        assert_eq!(texts(&effective_history(&history, None)), texts(&history));
    }

    #[test]
    fn effective_history_replaces_covered_messages_with_the_summary() {
        let history = conversation(3);
        let summary = StoredSummary { text: "earlier".into(), upto: 4 };
        let effective = effective_history(&history, Some(&summary));
        assert_eq!(effective.len(), 3);
        assert_eq!(effective[0].role, "summary");
        assert_eq!(effective[0].text, "earlier");
        assert_eq!(effective[1].text, "question 2");
        assert_eq!(effective[2].text, "answer 2");
    }

    #[test]
    fn effective_history_ignores_a_summary_beyond_the_history() {
        let history = conversation(1);
        let summary = StoredSummary { text: "stale".into(), upto: 6 };
        assert_eq!(texts(&effective_history(&history, Some(&summary))), texts(&history));
    }

    #[test]
    fn from_row_requires_text_and_a_positive_upto() {
        assert_eq!(
            StoredSummary::from_row(&json!({ "summary": "s", "summary_upto": 4 })),
            Some(StoredSummary { text: "s".into(), upto: 4 }),
        );
        assert_eq!(StoredSummary::from_row(&json!({ "summary": null, "summary_upto": 0 })), None);
        assert_eq!(StoredSummary::from_row(&json!({ "summary": "s", "summary_upto": 0 })), None);
    }

    #[tokio::test]
    async fn next_summary_skips_history_under_the_threshold() {
        let llm = RecordingLlm::new(Some("unused"));
        let agent = agent(llm.clone(), 10_000, 2);
        assert_eq!(next_summary(&agent, &conversation(5), None).await, None);
        assert!(llm.prompts().is_empty());
    }

    #[tokio::test]
    async fn next_summary_covers_all_but_the_kept_messages() {
        let llm = RecordingLlm::new(Some("summary v1"));
        let agent = agent(llm.clone(), 10, 2);
        let next = next_summary(&agent, &conversation(5), None).await;
        assert_eq!(next, Some(StoredSummary { text: "summary v1".into(), upto: 8 }));
    }

    #[tokio::test]
    async fn next_summary_folds_only_new_messages_into_the_previous_summary() {
        let llm = RecordingLlm::new(Some("summary v2"));
        let agent = agent(llm.clone(), 10, 2);
        let previous = StoredSummary { text: "summary v1".into(), upto: 6 };
        let next = next_summary(&agent, &conversation(6), Some(&previous)).await;
        assert_eq!(next, Some(StoredSummary { text: "summary v2".into(), upto: 10 }));
        let prompt = &llm.prompts()[0];
        assert!(prompt.contains("summary v1"));
        assert!(prompt.contains("question 3") && prompt.contains("answer 4"));
        assert!(!prompt.contains("question 2"), "messages already summarized must not be re-sent");
        assert!(!prompt.contains("question 5"), "kept messages must not be summarized");
    }

    #[tokio::test]
    async fn next_summary_waits_until_more_than_keep_last_new_messages_exist() {
        let llm = RecordingLlm::new(Some("unused"));
        let agent = agent(llm.clone(), 10, 2);
        let previous = StoredSummary { text: "x".repeat(100), upto: 8 };
        assert_eq!(next_summary(&agent, &conversation(5), Some(&previous)).await, None);
        assert!(llm.prompts().is_empty());
    }

    #[tokio::test]
    async fn next_summary_returns_none_when_the_llm_fails() {
        let agent = agent(RecordingLlm::new(None), 10, 2);
        assert_eq!(next_summary(&agent, &conversation(5), None).await, None);
    }

    async fn insert_conversation(db: &Db, messages: &[HistoryMessage]) -> (String, String) {
        let uid = uuid::Uuid::new_v4().to_string();
        let cid = uuid::Uuid::new_v4().to_string();
        db.query(
            "INSERT INTO users (id, provider) VALUES ($uid, 'local') RETURNING id",
            json!({ "uid": uid }),
        ).await.unwrap();
        let raw: Vec<Value> = messages.iter().map(|m| json!({ "role": m.role, "text": m.text })).collect();
        db.query(
            "INSERT INTO conversations (id, user_id, title, messages, message_count, created_at, updated_at)
             VALUES ($cid, $uid, 't', $messages, $count, now(), now()) RETURNING id",
            json!({ "cid": cid, "uid": uid, "messages": serde_json::to_string(&raw).unwrap(), "count": raw.len() as i64 }),
        ).await.unwrap();
        (uid, cid)
    }

    async fn stored(db: &Db, cid: &str) -> Option<StoredSummary> {
        let rows = db.query(
            "SELECT summary, summary_upto FROM conversations WHERE id = $cid",
            json!({ "cid": cid }),
        ).await.unwrap();
        StoredSummary::from_row(&rows[0])
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn refresh_stores_a_summary_that_the_next_load_uses() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let (uid, cid) = insert_conversation(&test_db.db, &conversation(5)).await;
        let agent = agent(RecordingLlm::new(Some("summary v1")), 10, 2);

        refresh(&test_db.db, &agent, &cid).await;

        assert_eq!(stored(&test_db.db, &cid).await, Some(StoredSummary { text: "summary v1".into(), upto: 8 }));
        let (raw, history) = crate::conversations::handlers::load_conversation_context(&test_db.db, &uid, &cid).await.unwrap();
        assert_eq!(raw.len(), 10);
        assert_eq!(texts(&history), vec![
            ("summary".to_string(), "summary v1".to_string()),
            ("user".to_string(), "question 4".to_string()),
            ("assistant".to_string(), "answer 4".to_string()),
        ]);
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn store_never_replaces_a_summary_with_one_covering_fewer_messages() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let (_, cid) = insert_conversation(&test_db.db, &conversation(5)).await;
        let newer = StoredSummary { text: "newer".into(), upto: 8 };
        let older = StoredSummary { text: "older".into(), upto: 4 };

        assert!(store(&test_db.db, &cid, &newer).await);
        assert!(!store(&test_db.db, &cid, &older).await);
        assert_eq!(stored(&test_db.db, &cid).await, Some(newer));
    }

    #[test]
    fn refresh_guard_allows_one_refresh_per_conversation() {
        let first = RefreshGuard::acquire("conv-guard-test");
        assert!(first.is_some());
        assert!(RefreshGuard::acquire("conv-guard-test").is_none());
        drop(first);
        assert!(RefreshGuard::acquire("conv-guard-test").is_some());
    }
}

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;

use super::{resolve_user_llm, QueryState};
use crate::agent::roles::REVIEWER;
use crate::code::jobs::store::{self, NewJob};
use crate::code::jobs::{Kind, Status};
use crate::code::workspace_state::{self, Snapshot};
use crate::code::{review, verification, CodeEnv};
use crate::conversations::handlers::append_user_turn;
use crate::cost::TurnUsage;
use crate::modes::Space;

pub struct FinishedTurn {
    pub space: Space,
    pub user_id: String,
    pub conversation_id: String,
    pub request: String,
    pub snapshot: Option<Snapshot>,
}

#[async_trait]
pub trait TurnHooks: Send + Sync {
    async fn before(&self, space: &Space) -> Option<Snapshot>;
    async fn after(&self, query: Arc<QueryState>, turn: FinishedTurn);
}

pub struct CodeTurnHooks {
    pub env: CodeEnv,
}

#[async_trait]
impl TurnHooks for CodeTurnHooks {
    async fn before(&self, space: &Space) -> Option<Snapshot> {
        let session_id = space.code_session_id()?;
        workspace_state::take(&self.env, session_id)
            .await
            .map_err(|e| tracing::warn!(session = session_id, error = %e, "could not snapshot the workspace"))
            .ok()
    }

    async fn after(&self, query: Arc<QueryState>, turn: FinishedTurn) {
        let (Some(session_id), Some(before)) = (turn.space.code_session_id(), &turn.snapshot) else {
            return;
        };
        let after = match workspace_state::take(&self.env, session_id).await {
            Ok(after) => after,
            Err(e) => {
                tracing::warn!(session = session_id, error = %e, "could not snapshot the workspace");
                return;
            }
        };
        let changes = workspace_state::between(before, &after);
        if changes.files {
            if let Err(e) = verification::start(&self.env, session_id, Some(&turn.conversation_id)).await {
                tracing::warn!(session = session_id, error = %e, "could not start the checks");
            }
        }
        if let Some((from, to)) = changes.commits.filter(|_| self.env.settings.review_changes) {
            let reviewer = Reviewer {
                env: self.env.clone(),
                query,
                session_id: session_id.to_string(),
                turn,
            };
            reviewer.spawn(from, to);
        }
    }
}

struct Reviewer {
    env: CodeEnv,
    query: Arc<QueryState>,
    session_id: String,
    turn: FinishedTurn,
}

impl Reviewer {
    fn spawn(self, from: String, to: String) {
        tokio::spawn(async move {
            if let Err(e) = self.run(&from, &to).await {
                tracing::warn!(session = self.session_id, error = %e, "the review failed");
            }
        });
    }

    async fn run(&self, from: &str, to: &str) -> Result<()> {
        let job = store::create(
            &self.env,
            NewJob {
                session_id: &self.session_id,
                chat_id: Some(&self.turn.conversation_id),
                kind: Kind::Review,
                label: review::LABEL,
            },
        )
        .await?;
        match self.review(from, to).await {
            Ok(findings) => store::finish(&self.env, &job.id, Status::Succeeded, None, &findings).await?,
            Err(e) => store::finish(&self.env, &job.id, Status::Failed, None, &format!("{e:#}")).await?,
        };
        Ok(())
    }

    async fn review(&self, from: &str, to: &str) -> Result<String> {
        let query = &self.query;
        let db = query.db.as_ref().context("reviews need the database")?;
        let llm = resolve_user_llm(&query.llm, &query.llm_configs, &query.user_key_store, &self.turn.user_id).await;
        let agent = query
            .agent_for_role(
                Space::code_session(&self.session_id, &self.turn.user_id),
                &self.turn.user_id,
                &self.turn.conversation_id,
                llm,
                &REVIEWER,
            )
            .await;
        let response = agent
            .query(&review::prompt(&self.turn.request, from, to), &[], &[], None)
            .await?;
        append_user_turn(
            db,
            &self.turn.user_id,
            &self.turn.conversation_id,
            review::REQUEST_TEXT,
            review::REVIEWER_NAME,
            &[],
            &response.answer,
            &response.sources,
            response.tool_calls_made,
            response.tool_errors,
            response.turns,
            vec![],
            None,
            None,
            &TurnUsage {
                provider_used: response.provider_used.as_ref(),
                usage: &response.usage,
                llm_call_count: response.llm_call_count,
                duration_ms: response.duration_ms,
                calls: &[],
            },
            &uuid::Uuid::new_v4().to_string(),
            &query.pricing,
        )
        .await?;
        Ok(response.answer)
    }
}

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::agent::Agent;
use crate::api::{resolve_user_llm, QueryState};
use crate::auth::api_keys::ApiKeyPrincipal;
use crate::conversations::handlers::{append_user_turn, load_conversation_context};
use crate::conversations::store::{self, Owner, Page};
use crate::llm::types::ProviderSelection;
use crate::turns::idempotency;

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

#[derive(Clone)]
pub struct ExternalChatState {
    pub qs: Arc<QueryState>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourcesMode {
    Raw,
    Url,
    Disabled,
}

impl Default for SourcesMode {
    fn default() -> Self {
        Self::Url
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagramsMode {
    Mermaid,
    Ascii,
    Disabled,
}

impl Default for DiagramsMode {
    fn default() -> Self {
        Self::Ascii
    }
}

#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(default)]
    pub tracking_id: Option<String>,
    #[serde(default)]
    pub sources: Option<SourcesMode>,
    #[serde(default)]
    pub diagrams: Option<DiagramsMode>,
    #[serde(default)]
    pub include_history: Option<bool>,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct HistoryQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    200
}

fn selection_from(req: &ChatRequest) -> Option<ProviderSelection> {
    req.provider_id
        .clone()
        .map(|provider_id| ProviderSelection {
            provider_id,
            model: req.model.clone(),
            cache_breakpoint_index: None,
        })
}

fn tracking_id() -> String {
    format!("trk_{}", Uuid::new_v4().simple())
}

fn conversation_id() -> String {
    Uuid::new_v4().to_string()
}

fn with_diagram_instruction(message: &str, diagrams: DiagramsMode) -> String {
    let instruction = match diagrams {
        DiagramsMode::Ascii => "If a diagram is useful, render it in Markdown using fenced ```text code blocks and ASCII-only characters.",
        DiagramsMode::Mermaid => "If a diagram is useful, render it in Markdown using fenced ```mermaid code blocks.",
        DiagramsMode::Disabled => "Do not include diagrams in the answer.",
    };
    format!("{}\n\n{}", message.trim(), instruction)
}

async fn resolve_user_name(db: &harvest_db::Db, user_id: &str) -> Option<String> {
    let rows = db
        .query(
            "SELECT name FROM users WHERE id = $id",
            json!({ "id": user_id }),
        )
        .await
        .ok()?;
    rows.first()
        .and_then(|row| row["name"].as_str().map(String::from))
}

async fn find_thread(db: &harvest_db::Db, api_key_id: &str, tracking_id: &str) -> Option<String> {
    let rows = db
        .query(
            "SELECT conversation_id FROM api_chat_threads
         WHERE api_key_id = $kid AND tracking_id = $tid",
            json!({ "kid": api_key_id, "tid": tracking_id }),
        )
        .await
        .ok()?;
    rows.first()
        .and_then(|row| row["conversation_id"].as_str().map(String::from))
}

async fn upsert_thread(
    db: &harvest_db::Db,
    api_key_id: &str,
    tracking_id: &str,
    conversation_id: &str,
) -> anyhow::Result<()> {
    db.query(
        "INSERT INTO api_chat_threads (id, api_key_id, tracking_id, conversation_id, created_at, updated_at)
         VALUES ($id, $kid, $tid, $cid, $now, $now)
         ON CONFLICT (api_key_id, tracking_id) DO UPDATE SET
             conversation_id = EXCLUDED.conversation_id,
             updated_at = EXCLUDED.updated_at",
        json!({
            "id": Uuid::new_v4().to_string(),
            "kid": api_key_id,
            "tid": tracking_id,
            "cid": conversation_id,
            "now": harvest_db::now_rfc3339(),
        }),
    ).await?;
    Ok(())
}

async fn mark_conversation_origin(
    db: &harvest_db::Db,
    user_id: &str,
    conversation_id: &str,
    api_key_id: &str,
    tracking_id: &str,
) -> anyhow::Result<()> {
    db.execute(
        "UPDATE conversations
         SET origin_type = 'api', origin_api_key_id = $kid, origin_tracking_id = $tid
         WHERE id = $cid AND user_id = $uid",
        json!({
            "kid": api_key_id,
            "tid": tracking_id,
            "cid": conversation_id,
            "uid": user_id,
        }),
    )
    .await?;
    Ok(())
}

async fn source_urls(db: &harvest_db::Db, sources: &[crate::agent::Source]) -> Vec<Value> {
    let mut out = Vec::new();
    for source in sources {
        let rows = db
            .query(
                "SELECT url FROM repositories WHERE name = $name",
                json!({ "name": source.repo }),
            )
            .await
            .unwrap_or_default();
        let repo_url = rows
            .first()
            .and_then(|row| row["url"].as_str())
            .unwrap_or_default();
        let Some(url) = build_source_url(repo_url, source) else {
            continue;
        };
        out.push(json!({
            "title": format!("{}:{}", source.file, source.line),
            "url": url,
        }));
    }
    out
}

fn build_source_url(repo_url: &str, source: &crate::agent::Source) -> Option<String> {
    if repo_url.is_empty() {
        return None;
    }
    let trimmed = repo_url.trim_end_matches(".git").trim_end_matches('/');
    if trimmed.contains("github.com") {
        return Some(format!(
            "{}/blob/{}/{}#L{}",
            trimmed, source.version, source.file, source.line
        ));
    }
    if trimmed.contains("gitlab") {
        return Some(format!(
            "{}/-/blob/{}/{}#L{}",
            trimmed, source.version, source.file, source.line
        ));
    }
    Some(format!(
        "{}/blob/{}/{}#L{}",
        trimmed, source.version, source.file, source.line
    ))
}

async fn history_for_thread(
    db: &harvest_db::Db,
    api_key_id: &str,
    tracking_id: &str,
    limit: usize,
) -> anyhow::Result<(String, Vec<Value>)> {
    let rows = db
        .query(
            "SELECT conversation_id AS cid FROM api_chat_threads
         WHERE api_key_id = $kid AND tracking_id = $tid",
            json!({ "kid": api_key_id, "tid": tracking_id }),
        )
        .await?;
    let Some(row) = rows.first() else {
        anyhow::bail!("not found");
    };
    let conversation_id = row["cid"].as_str().unwrap_or_default().to_string();
    let messages =
        store::load_page(db, Owner::Unscoped, &conversation_id, Page::last(limit)).await?;
    Ok((conversation_id, messages))
}

pub async fn chat(
    Extension(principal): Extension<ApiKeyPrincipal>,
    State(state): State<Arc<ExternalChatState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Result<axum::response::Response, ApiError> {
    let db = state
        .qs
        .db
        .as_ref()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let thread = req.tracking_id.clone().unwrap_or_default();
    let claim = idempotency::claim(db, &principal.user_id, &headers, &thread).await?;
    let answered = answer_chat(principal, Arc::clone(&state), req).await;
    if answered.is_err() {
        claim.release(db).await;
    }
    answered.map(IntoResponse::into_response)
}

async fn answer_chat(
    principal: ApiKeyPrincipal,
    state: Arc<ExternalChatState>,
    req: ChatRequest,
) -> Result<impl IntoResponse, ApiError> {
    if req.message.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "message must not be empty"));
    }

    let db = state
        .qs
        .db
        .as_ref()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let tracking_id = req.tracking_id.clone().unwrap_or_else(tracking_id);
    let include_history = req.include_history.unwrap_or(false);
    let sources_mode = req.sources.unwrap_or_default();
    let diagrams_mode = req.diagrams.unwrap_or_default();

    let conv_id = find_thread(db, &principal.key_id, &tracking_id)
        .await
        .unwrap_or_else(conversation_id);

    let history = load_conversation_context(db, &principal.user_id, &conv_id)
        .await
        .unwrap_or_default()
        .history();

    let selection = selection_from(&req);
    let user_llm = resolve_user_llm(
        &state.qs.llm,
        &state.qs.llm_configs,
        &state.qs.user_key_store,
        &principal.user_id,
    )
    .await;
    let agent = if Arc::ptr_eq(&user_llm, &state.qs.llm) {
        Arc::clone(&state.qs.agent)
    } else {
        Arc::new(
            Agent::new(
                user_llm,
                crate::agent::graph_tools::all_tools_with_semantic(
                    Arc::clone(db),
                    state.qs.semantic.clone(),
                ),
                state.qs.max_iterations,
            )
            .with_compaction(
                state.qs.compaction_threshold_chars,
                state.qs.compaction_keep_last,
            )
            .with_parallel_research(true),
        )
    };

    let message = with_diagram_instruction(&req.message, diagrams_mode);

    let response = agent
        .query(&message, &history, &[], selection.as_ref())
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "query failed"))?;

    let username = resolve_user_name(db, &principal.user_id)
        .await
        .unwrap_or_else(|| principal.user_id.clone());
    let turn_id = Uuid::new_v4().to_string();
    append_user_turn(
        db,
        &principal.user_id,
        &conv_id,
        &req.message,
        &username,
        &[],
        &response.answer,
        &response.sources,
        response.tool_calls_made,
        response.tool_errors,
        response.turns,
        vec![],
        None,
        None,
        response.provider_used.as_ref(),
        response.duration_ms,
        &response.usage,
        response.llm_call_count,
        &turn_id,
        &state.qs.pricing,
    )
    .await
    .map_err(|_| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to persist conversation",
        )
    })?;

    upsert_thread(db, &principal.key_id, &tracking_id, &conv_id)
        .await
        .map_err(|_| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to persist conversation",
            )
        })?;
    mark_conversation_origin(
        db,
        &principal.user_id,
        &conv_id,
        &principal.key_id,
        &tracking_id,
    )
    .await
    .map_err(|_| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to persist conversation",
        )
    })?;

    let mut body = json!({
        "tracking_id": tracking_id,
        "conversation_id": conv_id,
        "response": response.answer,
        "usage": response.usage,
        "tool_calls_made": response.tool_calls_made,
        "tool_errors": response.tool_errors,
        "turns": response.turns,
        "duration_ms": response.duration_ms,
        "llm_call_count": response.llm_call_count,
        "cost_microusd": state.qs.pricing.price_call(
            &response.usage,
            response.provider_used.as_ref().map(|p| p.kind.as_str()).unwrap_or(""),
            response.provider_used.as_ref().map(|p| p.model.as_str()).unwrap_or(""),
        ),
    });

    if let Some(provider_used) = response.provider_used.as_ref() {
        body["provider"] = json!({
            "provider_id": provider_used.provider_id,
            "kind": provider_used.kind,
            "model": provider_used.model,
        });
    }

    match sources_mode {
        SourcesMode::Disabled => {}
        SourcesMode::Raw => {
            body["sources"] = serde_json::to_value(response.sources).unwrap_or_else(|_| json!([]));
        }
        SourcesMode::Url => {
            body["sources"] = json!(source_urls(db, &response.sources).await);
        }
    }

    if include_history {
        let (_, history) = history_for_thread(
            db,
            &principal.key_id,
            body["tracking_id"].as_str().unwrap_or_default(),
            10_000,
        )
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "failed to load history"))?;
        body["history"] = json!(history);
    }

    Ok(Json(body))
}

pub async fn history(
    Extension(principal): Extension<ApiKeyPrincipal>,
    State(state): State<Arc<ExternalChatState>>,
    Path(tracking_id): Path<String>,
    Query(params): Query<HistoryQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .qs
        .db
        .as_ref()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let limit = params.limit.clamp(1, 1000);
    match history_for_thread(db, &principal.key_id, &tracking_id, limit).await {
        Ok((conversation_id, history)) => Ok(Json(json!({
            "tracking_id": tracking_id,
            "conversation_id": conversation_id,
            "history": history,
        }))),
        Err(_) => Err(err(StatusCode::NOT_FOUND, "not found")),
    }
}

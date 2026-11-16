use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde_json::{json, Value};

use super::chats::{self, NEW_CHAT_TITLE};
use super::handlers::{env, failure, internal, session_for, CodeState, Reply};
use super::sessions;
use crate::auth::jwt::Claims;

const LAYOUT_LIMIT: usize = 64 * 1024;

pub async fn list(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(session_id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let chats = chats::list(&env.db, &session_id).await.map_err(internal)?;
    Ok(Json(json!({ "chats": chats })).into_response())
}

pub async fn create(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(session_id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let chat = chats::create(&env.db, &session_id, &user.sub, NEW_CHAT_TITLE)
        .await
        .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({ "chat": chat }))).into_response())
}

pub async fn delete(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path((session_id, chat_id)): Path<(String, String)>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    if !chats::delete(&env.db, &session_id, &chat_id).await.map_err(internal)? {
        return Err(failure(StatusCode::NOT_FOUND, "not found"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn layout(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(session_id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let layout = sessions::layout(env, &session_id).await.map_err(internal)?;
    Ok(Json(json!({ "layout": layout })).into_response())
}

pub async fn save_layout(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(session_id): Path<String>,
    Json(body): Json<Value>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let layout = &body["layout"];
    if !layout.is_object() || layout.to_string().len() > LAYOUT_LIMIT {
        return Err(failure(StatusCode::BAD_REQUEST, "the layout must be an object of at most 64 KiB"));
    }
    sessions::save_layout(env, &session_id, layout).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;

use super::super::handlers::{env, failure, internal, session_for, CodeState, Reply};
use super::super::{sessions, CodeEnv};
use super::{Audience, Kind, Preview, Settings, Slot};
use crate::auth::jwt::Claims;

async fn owned_preview(env: &CodeEnv, user: &Claims, preview_id: &str) -> Result<Preview, Response> {
    let preview = super::get(&env.db, preview_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "not found"))?;
    session_for(env, user, &preview.session_id).await?;
    Ok(preview)
}

pub async fn list(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(session_id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let previews = super::list(&env.db, &session_id, env.settings.preview_lifetime)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "previews": previews })).into_response())
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub kind: String,
    pub port: Option<u16>,
    pub start_command: Option<String>,
}

pub async fn create(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(session_id): Path<String>,
    Json(body): Json<CreateBody>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let kind = Kind::parse(&body.kind).ok_or_else(|| failure(StatusCode::BAD_REQUEST, "kind must be web, api or cli"))?;
    let container = sessions::active_container(env, &session_id)
        .await
        .map_err(|e| failure(StatusCode::CONFLICT, e.to_string()))?;
    let settings = Settings { kind, port: body.port, start_command: body.start_command.as_deref() };
    let preview = super::configure(&env.db, &session_id, None, Slot::New, settings)
        .await
        .map_err(|e| failure(StatusCode::BAD_REQUEST, e.to_string()))?;
    if let Err(e) = super::apply(env, &container, &preview).await {
        let _ = super::delete(env, &preview).await;
        return Err(failure(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()));
    }
    Ok((StatusCode::CREATED, Json(json!({ "preview": preview }))).into_response())
}

pub async fn delete(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(preview_id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    let preview = owned_preview(env, &user, &preview_id).await?;
    super::delete(env, &preview).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
pub struct ShareBody {
    pub audience: String,
}

pub async fn share(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(preview_id): Path<String>,
    Json(body): Json<ShareBody>,
) -> Reply {
    let env = env(&state)?;
    owned_preview(env, &user, &preview_id).await?;
    let audience = Audience::parse(&body.audience)
        .ok_or_else(|| failure(StatusCode::BAD_REQUEST, "choose owner, repository or link"))?;
    super::set_audience(&env.db, &preview_id, audience).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

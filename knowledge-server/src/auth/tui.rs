use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use super::{ephemeral, jwt, AuthState, TOKEN_COOKIE};
use axum_extra::extract::cookie::CookieJar;

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

const TUI_AUTH_TTL: Duration = Duration::from_secs(300);

pub async fn create_auth_request(
    State(state): State<Arc<AuthState>>,
) -> Result<impl IntoResponse, ApiError> {
    let uuid = Uuid::new_v4().to_string();
    let public_url = state
        .config
        .public_url
        .clone()
        .unwrap_or_else(|| format!("http://{}:{}", "localhost", "8080"));
    let auth_url = format!("{}/#/authenticate/{}", public_url, uuid);

    ephemeral::put(&state.db, ephemeral::TUI_KIND, &uuid, &json!({ "status": "pending" }), TUI_AUTH_TTL)
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "could not create the auth request"))?;

    Ok(Json(json!({
        "uuid": uuid,
        "auth_url": auth_url,
    })))
}

pub async fn poll_auth_request(
    State(state): State<Arc<AuthState>>,
    Path(uuid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let entry = ephemeral::get(&state.db, ephemeral::TUI_KIND, &uuid)
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "server error"))?;

    let Some(entry) = entry else {
        return Ok(Json(json!({ "status": "expired" })));
    };

    match entry["status"].as_str() {
        Some("authorized") => {
            let _ = ephemeral::take(&state.db, ephemeral::TUI_KIND, &uuid).await;
            Ok(Json(json!({
                "status": "authorized",
                "token": entry["token"],
                "email": entry["user_email"],
            })))
        }
        Some("denied") => {
            let _ = ephemeral::take(&state.db, ephemeral::TUI_KIND, &uuid).await;
            Ok(Json(json!({ "status": "denied" })))
        }
        _ => Ok(Json(json!({ "status": "pending" }))),
    }
}

#[derive(serde::Deserialize)]
pub struct AuthorizeBody {
    #[serde(default = "default_true")]
    pub approved: bool,
}

fn default_true() -> bool {
    true
}

pub async fn authorize_request(
    State(state): State<Arc<AuthState>>,
    Path(uuid): Path<String>,
    jar: CookieJar,
    body: Option<Json<AuthorizeBody>>,
) -> Result<impl IntoResponse, ApiError> {
    let token = jar
        .get(TOKEN_COOKIE)
        .map(|c| c.value().to_string())
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "unauthorized"))?;

    let claims = jwt::validate(&state.config.jwt_secret, &token)
        .map_err(|_| err(StatusCode::UNAUTHORIZED, "unauthorized"))?;

    let approved = body.map(|b| b.approved).unwrap_or(true);

    let next = if approved {
        let tui_token = jwt::issue(
            &state.config.jwt_secret,
            &claims.sub,
            &claims.email,
            &claims.name,
            &claims.role,
        )
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
        json!({ "status": "authorized", "token": tui_token, "user_email": claims.email })
    } else {
        json!({ "status": "denied" })
    };

    let updated = ephemeral::replace_if(&state.db, ephemeral::TUI_KIND, &uuid, |current| {
        (current["status"].as_str() == Some("pending")).then(|| next.clone())
    })
    .await
    .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "server error"))?;

    match updated {
        Some(value) => Ok(Json(json!({ "status": value["status"] }))),
        None => Err(err(StatusCode::NOT_FOUND, "auth request not found or expired")),
    }
}

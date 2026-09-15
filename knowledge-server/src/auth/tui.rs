use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use super::{jwt, AuthState, TOKEN_COOKIE};
use axum_extra::extract::cookie::CookieJar;

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

#[derive(Clone)]
pub struct TuiAuthEntry {
    pub status: String,
    pub token: Option<String>,
    pub user_email: Option<String>,
    pub created_at: Instant,
}

pub type TuiAuthMap = Arc<dashmap::DashMap<String, TuiAuthEntry>>;

pub fn new_auth_map() -> TuiAuthMap {
    Arc::new(dashmap::DashMap::new())
}

const TUI_AUTH_TTL_SECS: u64 = 300;

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

    let map = get_tui_map(&state);
    map.insert(
        uuid.clone(),
        TuiAuthEntry {
            status: "pending".to_string(),
            token: None,
            user_email: None,
            created_at: Instant::now(),
        },
    );

    Ok(Json(json!({
        "uuid": uuid,
        "auth_url": auth_url,
    })))
}

fn get_tui_map(state: &AuthState) -> TuiAuthMap {
    state.tui_auth.clone()
}

pub async fn poll_auth_request(
    State(state): State<Arc<AuthState>>,
    Path(uuid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let map = get_tui_map(&state);

    let entry = map.get(&uuid).filter(|e| e.created_at.elapsed().as_secs() < TUI_AUTH_TTL_SECS);

    match entry {
        Some(e) => {
            let status = e.status.clone();
            match status.as_str() {
                "authorized" => {
                    let token = e.token.clone();
                    let email = e.user_email.clone();
                    drop(e);
                    map.remove(&uuid);
                    Ok(Json(json!({
                        "status": "authorized",
                        "token": token,
                        "email": email,
                    })))
                }
                "denied" => {
                    drop(e);
                    map.remove(&uuid);
                    Ok(Json(json!({ "status": "denied" })))
                }
                _ => Ok(Json(json!({ "status": "pending" }))),
            }
        }
        None => Ok(Json(json!({ "status": "expired" }))),
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

    let map = get_tui_map(&state);

    let entry = map.get_mut(&uuid).filter(|e| {
        e.status == "pending" && e.created_at.elapsed().as_secs() < TUI_AUTH_TTL_SECS
    });

    match entry {
        Some(mut e) => {
            if approved {
                let tui_token = jwt::issue(
                    &state.config.jwt_secret,
                    &claims.sub,
                    &claims.email,
                    &claims.name,
                    &claims.role,
                )
                .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

                e.status = "authorized".to_string();
                e.token = Some(tui_token);
                e.user_email = Some(claims.email.clone());

                Ok(Json(json!({ "status": "authorized" })))
            } else {
                e.status = "denied".to_string();
                Ok(Json(json!({ "status": "denied" })))
            }
        }
        None => Err(err(StatusCode::NOT_FOUND, "auth request not found or expired")),
    }
}

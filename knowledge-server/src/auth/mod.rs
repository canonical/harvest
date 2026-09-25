pub mod handlers;
pub mod jwt;
pub mod oidc;
pub mod password;
pub mod tui;
pub mod user_keys;

use axum::{extract::Request, http::StatusCode, middleware::Next, response::IntoResponse, Json};
use dashmap::DashMap;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

use crate::config::{AuthConfig, UiConfig};
use harvest_db::Db;

pub use oidc::OidcEndpoints;
pub use tui::TuiAuthMap;

pub const TOKEN_COOKIE: &str = "token";

pub struct OAuthSession {
    pub pkce_verifier: Option<String>,
    pub created_at:    Instant,
}

pub type OAuthSessions = Arc<DashMap<String, OAuthSession>>;

#[derive(Clone)]
pub struct AuthState {
    pub db:          Arc<Db>,
    pub config:         Arc<AuthConfig>,
    pub ui:             Arc<UiConfig>,
    pub http:           reqwest::Client,
    pub oidc_endpoints: Option<Arc<OidcEndpoints>>,
    pub oauth_sessions: OAuthSessions,
    pub lxd_enabled:    bool,
    pub tui_auth:       TuiAuthMap,
}

fn token_from_request(req: &Request) -> Option<String> {
    if let Some(auth) = req.headers().get(axum::http::header::AUTHORIZATION) {
        if let Ok(s) = auth.to_str() {
            if let Some(bearer) = s.strip_prefix("Bearer ") {
                return Some(bearer.to_string());
            }
        }
    }
    req.headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies
                .split(';')
                .map(|c| c.trim())
                .find(|c| c.starts_with("token="))
                .map(|c| c["token=".len()..].to_string())
        })
}

pub async fn require_auth(
    axum::extract::State(secret): axum::extract::State<Arc<String>>,
    mut req: Request,
    next: Next,
) -> impl IntoResponse {
    let token = token_from_request(&req);
    match token.and_then(|t| jwt::validate(&secret, &t).ok()) {
        Some(claims) => {
            req.extensions_mut().insert(claims);
            next.run(req).await.into_response()
        }
        None => (StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorized" }))).into_response(),
    }
}

pub async fn require_admin(
    axum::extract::State(secret): axum::extract::State<Arc<String>>,
    req: Request,
    next: Next,
) -> impl IntoResponse {
    match token_from_request(&req).and_then(|t| jwt::validate(&secret, &t).ok()) {
        Some(claims) if claims.role == "admin" => next.run(req).await.into_response(),
        Some(_) => (StatusCode::FORBIDDEN, Json(json!({ "error": "forbidden" }))).into_response(),
        None => (StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorized" }))).into_response(),
    }
}

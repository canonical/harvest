pub mod handlers;
pub mod jwt;
pub mod oidc;
pub mod password;

use anyhow::Result;
use axum::{extract::Request, http::StatusCode, middleware::Next, response::IntoResponse, Json};
use dashmap::DashMap;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

use crate::config::{AuthConfig, UiConfig};
use crate::neo4j::Neo4jClient;

pub use oidc::OidcEndpoints;

pub const TOKEN_COOKIE: &str = "token";

pub struct OAuthSession {
    pub pkce_verifier: Option<String>,
    pub created_at:    Instant,
}

pub type OAuthSessions = Arc<DashMap<String, OAuthSession>>;

#[derive(Clone)]
pub struct AuthState {
    pub neo4j:          Arc<Neo4jClient>,
    pub config:         Arc<AuthConfig>,
    pub ui:             Arc<UiConfig>,
    pub http:           reqwest::Client,
    pub oidc_endpoints: Option<Arc<OidcEndpoints>>,
    pub oauth_sessions: OAuthSessions,
}

pub async fn setup_constraints(neo4j: &Neo4jClient) -> Result<()> {
    neo4j.run("CREATE CONSTRAINT user_email IF NOT EXISTS FOR (u:User) REQUIRE u.email IS UNIQUE").await?;
    neo4j.run("CREATE CONSTRAINT user_google_id IF NOT EXISTS FOR (u:User) REQUIRE u.google_id IS UNIQUE").await?;
    neo4j.run("CREATE CONSTRAINT user_oidc_sub IF NOT EXISTS FOR (u:User) REQUIRE u.oidc_sub IS UNIQUE").await?;
    neo4j.run("CREATE CONSTRAINT group_id IF NOT EXISTS FOR (g:Group) REQUIRE g.id IS UNIQUE").await?;
    Ok(())
}

fn token_from_request(req: &Request) -> Option<String> {
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

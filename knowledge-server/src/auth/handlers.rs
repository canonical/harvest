use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use time::Duration;
use uuid::Uuid;

use super::{jwt, oidc, password, AuthState, TOKEN_COOKIE};
use crate::config::{GoogleConfig, OidcConfig};

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

fn make_token_cookie(token: String) -> Cookie<'static> {
    Cookie::build((TOKEN_COOKIE, token))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(Duration::days(30))
        .build()
}

fn clear_token_cookie() -> Cookie<'static> {
    Cookie::build((TOKEN_COOKIE, ""))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(Duration::seconds(0))
        .build()
}

pub async fn config(State(state): State<Arc<AuthState>>) -> impl IntoResponse {
    let oidc_display_name = state.config.oidc.as_ref()
        .and_then(|o| o.display_name.clone());
    Json(serde_json::json!({
        "local_login": state.config.allow_local_login,
        "google": state.config.google.is_some(),
        "oidc":   state.oidc_endpoints.is_some(),
        "oidc_display_name": oidc_display_name,
    }))
}

#[derive(Deserialize)]
pub struct RegisterBody {
    pub email: String,
    pub name: String,
    pub password: String,
}

pub async fn register(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
    Json(body): Json<RegisterBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !state.config.allow_local_login {
        return Err(err(StatusCode::FORBIDDEN, "local login is disabled"));
    }
    if body.email.is_empty() || body.password.len() < 8 || body.name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "invalid input"));
    }

    let hash = password::hash(&body.password).map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let rows = state.neo4j.query_read(
        "MATCH (existing:User)
         WITH count(existing) AS n
         CREATE (u:User {
           id: $id, email: $email, name: $name,
           password_hash: $password_hash, provider: 'local',
           role: CASE WHEN n = 0 THEN 'admin' ELSE 'regular' END,
           created_at: $created_at
         })
         RETURN u.id AS id, u.email AS email, u.name AS name, u.role AS role",
        json!({ "id": id, "email": body.email, "name": body.name,
                "password_hash": hash, "created_at": now }),
    ).await.map_err(|e| {
        if e.to_string().contains("already exists") || e.to_string().contains("ConstraintValidationFailed") {
            err(StatusCode::CONFLICT, "email already registered")
        } else {
            err(StatusCode::INTERNAL_SERVER_ERROR, "server error")
        }
    })?;

    let user = rows.into_iter().next().ok_or_else(|| err(StatusCode::CONFLICT, "email already registered"))?;
    let token = issue_token(&state.config.jwt_secret, &user)?;

    Ok((jar.add(make_token_cookie(token)), Json(json!({ "ok": true }))))
}

#[derive(Deserialize)]
pub struct LoginBody {
    pub email: String,
    pub password: String,
}

pub async fn login(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
    Json(body): Json<LoginBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !state.config.allow_local_login {
        return Err(err(StatusCode::FORBIDDEN, "local login is disabled"));
    }
    let rows = state.neo4j.query_read(
        "MATCH (u:User {email: $email, provider: 'local'})
         RETURN u.id AS id, u.email AS email, u.name AS name,
                u.role AS role, u.password_hash AS password_hash",
        json!({ "email": body.email }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    let user = rows.into_iter().next().ok_or_else(|| err(StatusCode::UNAUTHORIZED, "invalid credentials"))?;
    let hash = user["password_hash"].as_str().unwrap_or("");

    let ok = password::verify(&body.password, hash)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if !ok {
        return Err(err(StatusCode::UNAUTHORIZED, "invalid credentials"));
    }

    let token = issue_token(&state.config.jwt_secret, &user)?;
    Ok((jar.add(make_token_cookie(token)), Json(json!({ "ok": true }))))
}

pub async fn logout(jar: CookieJar) -> impl IntoResponse {
    (jar.add(clear_token_cookie()), Json(json!({ "ok": true })))
}

#[derive(Serialize)]
pub struct MeResponse {
    pub id: String,
    pub email: String,
    pub name: String,
    pub role: String,
    pub last_project_id: Option<String>,
}

pub async fn me(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&state.config.jwt_secret, &jar)?;
    let rows = state.neo4j.query_read(
        "MATCH (u:User {id: $id}) RETURN u.last_project_id AS last_project_id",
        json!({ "id": claims.sub }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let last_project_id = rows.into_iter()
        .next()
        .and_then(|r| r["last_project_id"].as_str().map(String::from));
    Ok(Json(MeResponse {
        id: claims.sub,
        email: claims.email,
        name: claims.name,
        role: claims.role,
        last_project_id,
    }))
}

#[derive(Deserialize)]
pub struct UpdateMeBody {
    pub last_project_id: String,
}

pub async fn update_me(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
    Json(body): Json<UpdateMeBody>,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&state.config.jwt_secret, &jar)?;
    state.neo4j.query_read(
        "MATCH (u:User {id: $id}) SET u.last_project_id = $pid RETURN u.id",
        json!({ "id": claims.sub, "pid": body.last_project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn google_redirect(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
) -> Result<impl IntoResponse, ApiError> {
    let google = state.config.google.as_ref().ok_or_else(|| err(StatusCode::NOT_IMPLEMENTED, "Google login not configured"))?;

    let oauth_state = Uuid::new_v4().to_string();
    let url = build_google_auth_url(google, &oauth_state);
    tracing::info!(redirect_uri = %google.redirect_uri, auth_url = %url, "Initiating Google OAuth");

    let state_cookie = Cookie::build(("oauth_state", oauth_state))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(Duration::minutes(10))
        .build();

    Ok((jar.add(state_cookie), Redirect::to(&url)))
}

#[derive(Deserialize)]
pub struct GoogleCallbackParams {
    pub code:              Option<String>,
    pub state:             Option<String>,
    pub error:             Option<String>,
    pub error_description: Option<String>,
}

pub async fn google_callback(
    State(state): State<Arc<AuthState>>,
    Query(params): Query<GoogleCallbackParams>,
    jar: CookieJar,
) -> Result<impl IntoResponse, ApiError> {
    let google = state.config.google.as_ref().ok_or_else(|| err(StatusCode::NOT_IMPLEMENTED, "Google login not configured"))?;

    if let Some(e) = params.error {
        let desc = params.error_description.unwrap_or_default();
        tracing::warn!(error = %e, description = %desc, "Google OAuth error");
        return Err(err(StatusCode::BAD_REQUEST, &format!("Google OAuth error: {e}")));
    }

    let code = params.code.ok_or_else(|| err(StatusCode::BAD_REQUEST, "missing OAuth code"))?;
    let oauth_state = params.state.ok_or_else(|| err(StatusCode::BAD_REQUEST, "missing OAuth state"))?;

    let stored = jar.get("oauth_state").map(|c| c.value().to_string());
    if stored.as_deref() != Some(oauth_state.as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, "invalid OAuth state"));
    }

    let access_token = exchange_google_code(&state.http, google, &code)
        .await
        .map_err(|_| err(StatusCode::BAD_GATEWAY, "failed to exchange OAuth code"))?;

    let google_user = fetch_google_user(&state.http, &access_token)
        .await
        .map_err(|_| err(StatusCode::BAD_GATEWAY, "failed to fetch Google user info"))?;

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let rows = state.neo4j.query_read(
        "MATCH (existing:User)
         WITH count(existing) AS n
         MERGE (u:User {google_id: $google_id})
         ON CREATE SET u.id = $id, u.email = $email, u.name = $name,
           u.provider = 'google', u.created_at = $created_at,
           u.role = CASE WHEN n = 0 THEN 'admin' ELSE 'regular' END
         ON MATCH SET u.email = $email, u.name = $name
         RETURN u.id AS id, u.email AS email, u.name AS name, u.role AS role",
        json!({
            "google_id": google_user.id,
            "id": id,
            "email": google_user.email,
            "name": google_user.name,
            "created_at": now
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    let user = rows.into_iter().next().ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let token = issue_token(&state.config.jwt_secret, &user)?;

    let remove_state = Cookie::build(("oauth_state", ""))
        .path("/")
        .max_age(Duration::seconds(0))
        .build();

    Ok((
        jar.add(make_token_cookie(token)).add(remove_state),
        Redirect::to("/"),
    ))
}

fn extract_claims(secret: &str, jar: &CookieJar) -> Result<jwt::Claims, ApiError> {
    let token = jar
        .get(TOKEN_COOKIE)
        .map(|c| c.value().to_string())
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "unauthorized"))?;
    jwt::validate(secret, &token).map_err(|_| err(StatusCode::UNAUTHORIZED, "unauthorized"))
}

fn issue_token(secret: &str, user: &Value) -> Result<String, ApiError> {
    jwt::issue(
        secret,
        user["id"].as_str().unwrap_or(""),
        user["email"].as_str().unwrap_or(""),
        user["name"].as_str().unwrap_or(""),
        user["role"].as_str().unwrap_or("regular"),
    )
    .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))
}

pub async fn oidc_redirect(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
) -> Result<impl IntoResponse, ApiError> {
    let endpoints = state.oidc_endpoints.as_ref()
        .ok_or_else(|| err(StatusCode::NOT_IMPLEMENTED, "OIDC login not configured"))?;
    let oidc_cfg = state.config.oidc.as_ref()
        .ok_or_else(|| err(StatusCode::NOT_IMPLEMENTED, "OIDC login not configured"))?;

    let oauth_state = Uuid::new_v4().to_string();
    let (pkce_verifier, pkce_challenge) = oidc::generate_pkce_pair();
    let url = build_oidc_auth_url(&endpoints.authorization_endpoint, oidc_cfg, &oauth_state, &pkce_challenge);

    let state_cookie = Cookie::build(("oauth_state", oauth_state))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(Duration::minutes(10))
        .build();

    let pkce_cookie = Cookie::build(("pkce_verifier", pkce_verifier))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(Duration::minutes(10))
        .build();

    Ok((jar.add(state_cookie).add(pkce_cookie), Redirect::to(&url)))
}

#[derive(Deserialize)]
pub struct OidcCallbackParams {
    pub code:              Option<String>,
    pub state:             Option<String>,
    pub error:             Option<String>,
    pub error_description: Option<String>,
}

pub async fn oidc_callback(
    State(state): State<Arc<AuthState>>,
    Query(params): Query<OidcCallbackParams>,
    jar: CookieJar,
) -> Result<impl IntoResponse, ApiError> {
    let endpoints = state.oidc_endpoints.as_ref()
        .ok_or_else(|| err(StatusCode::NOT_IMPLEMENTED, "OIDC login not configured"))?;
    let oidc_cfg = state.config.oidc.as_ref()
        .ok_or_else(|| err(StatusCode::NOT_IMPLEMENTED, "OIDC login not configured"))?;

    if let Some(e) = params.error {
        let desc = params.error_description.unwrap_or_default();
        tracing::warn!(error = %e, description = %desc, "OIDC error from provider");
        return Err(err(StatusCode::BAD_REQUEST, &format!("OIDC error: {e}")));
    }

    let code = params.code.ok_or_else(|| err(StatusCode::BAD_REQUEST, "missing OAuth code"))?;
    let oauth_state = params.state.ok_or_else(|| err(StatusCode::BAD_REQUEST, "missing OAuth state"))?;

    let stored = jar.get("oauth_state").map(|c| c.value().to_string());
    if stored.as_deref() != Some(oauth_state.as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, "invalid OAuth state"));
    }

    let pkce_verifier = jar.get("pkce_verifier").map(|c| c.value().to_string());

    let access_token = oidc::exchange_code(
        &state.http,
        &endpoints.token_endpoint,
        &oidc_cfg.client_id,
        &oidc_cfg.client_secret,
        &oidc_cfg.redirect_uri,
        &code,
        pkce_verifier.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "OIDC code exchange failed");
        err(StatusCode::BAD_GATEWAY, "failed to exchange OIDC code")
    })?;

    let user_info = oidc::fetch_userinfo(&state.http, &endpoints.userinfo_endpoint, &access_token)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "OIDC userinfo fetch failed");
            err(StatusCode::BAD_GATEWAY, "failed to fetch OIDC user info")
        })?;

    let display_name = user_info.name.unwrap_or_else(|| {
        user_info.email.split('@').next().unwrap_or("user").to_string()
    });

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let rows = state.neo4j.query_read(
        "MATCH (existing:User)
         WITH count(existing) AS n
         MERGE (u:User {email: $email})
         ON CREATE SET u.id = $id, u.name = $name,
           u.provider = 'oidc', u.created_at = $created_at,
           u.role = CASE WHEN n = 0 THEN 'admin' ELSE 'regular' END
         ON MATCH SET u.name = $name
         SET u.oidc_sub = $oidc_sub
         RETURN u.id AS id, u.email AS email, u.name AS name, u.role AS role",
        serde_json::json!({
            "oidc_sub": user_info.sub,
            "id": id,
            "email": user_info.email,
            "name": display_name,
            "created_at": now
        }),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "OIDC user upsert failed");
        err(StatusCode::INTERNAL_SERVER_ERROR, "server error")
    })?;

    let user = rows.into_iter().next()
        .ok_or_else(|| {
            tracing::error!("OIDC user upsert returned no rows");
            err(StatusCode::INTERNAL_SERVER_ERROR, "server error")
        })?;
    let token = issue_token(&state.config.jwt_secret, &user)?;

    let remove_state = Cookie::build(("oauth_state", ""))
        .path("/").max_age(Duration::seconds(0)).build();
    let remove_pkce = Cookie::build(("pkce_verifier", ""))
        .path("/").max_age(Duration::seconds(0)).build();

    Ok((
        jar.add(make_token_cookie(token)).add(remove_state).add(remove_pkce),
        Redirect::to("/"),
    ))
}

fn build_oidc_auth_url(
    authorization_endpoint: &str,
    oidc: &OidcConfig,
    state: &str,
    pkce_challenge: &str,
) -> String {
    reqwest::Url::parse_with_params(
        authorization_endpoint,
        &[
            ("client_id", oidc.client_id.as_str()),
            ("redirect_uri", oidc.redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", "openid email profile"),
            ("state", state),
            ("code_challenge", pkce_challenge),
            ("code_challenge_method", "S256"),
        ],
    )
    .expect("valid OIDC auth URL")
    .to_string()
}

fn build_google_auth_url(google: &GoogleConfig, state: &str) -> String {
    reqwest::Url::parse_with_params(
        "https://accounts.google.com/o/oauth2/v2/auth",
        &[
            ("client_id", google.client_id.as_str()),
            ("redirect_uri", google.redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", "openid email profile"),
            ("state", state),
            ("access_type", "offline"),
        ],
    )
    .expect("valid Google auth URL")
    .to_string()
}

#[derive(Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct GoogleUserInfo {
    id: String,
    email: String,
    name: String,
}

async fn exchange_google_code(
    http: &reqwest::Client,
    google: &GoogleConfig,
    code: &str,
) -> anyhow::Result<String> {
    let resp = http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", code),
            ("client_id", &google.client_id),
            ("client_secret", &google.client_secret),
            ("redirect_uri", &google.redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await?
        .json::<GoogleTokenResponse>()
        .await?;
    Ok(resp.access_token)
}

async fn fetch_google_user(
    http: &reqwest::Client,
    access_token: &str,
) -> anyhow::Result<GoogleUserInfo> {
    Ok(http
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .bearer_auth(access_token)
        .send()
        .await?
        .json::<GoogleUserInfo>()
        .await?)
}

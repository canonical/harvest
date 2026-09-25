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

use super::{jwt, oidc, password, AuthState, OAuthSession, TOKEN_COOKIE};
use crate::config::{GoogleConfig, OidcConfig};
use harvest_db::Db;

const SESSION_TTL_SECS: u64 = 600; // 10 minutes

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
        "features": {
            "docs": state.ui.enable_docs,
            "lxd":  state.lxd_enabled,
        },
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
    let now = harvest_db::now_rfc3339();

    let rows = insert_user(&state.db,
        "INSERT INTO users (id, email, name, password_hash, provider, role, created_at)
         VALUES ($id, $email, $name, $password_hash, 'local',
                 CASE WHEN EXISTS (SELECT 1 FROM users) THEN 'regular' ELSE 'admin' END, $created_at)
         RETURNING id, email, name, role",
        json!({ "id": id, "email": body.email, "name": body.name,
                "password_hash": hash, "created_at": now }),
    ).await.map_err(|e| {
        if harvest_db::is_unique_violation(&e) {
            err(StatusCode::CONFLICT, "email already registered")
        } else {
            err(StatusCode::INTERNAL_SERVER_ERROR, "server error")
        }
    })?;

    let user = rows.into_iter().next().ok_or_else(|| err(StatusCode::CONFLICT, "email already registered"))?;
    assign_default_groups(&state, &id).await?;
    let token = issue_token(&state.config.jwt_secret, &user)?;

    Ok((jar.add(make_token_cookie(token)), Json(json!({ "ok": true }))))
}

/// Serialises user creation so exactly one account can become the first admin.
async fn insert_user(db: &Db, sql: &str, params: Value) -> anyhow::Result<Vec<Value>> {
    let tx = db.begin().await?;
    tx.execute("SELECT pg_advisory_xact_lock(hashtext('harvest:first-user'))", json!({})).await?;
    let rows = tx.query(sql, params).await?;
    tx.commit().await?;
    Ok(rows)
}

async fn assign_default_groups(state: &AuthState, user_id: &str) -> Result<(), ApiError> {
    let rows = state.db.query(
        "WITH defaults AS (SELECT id FROM groups WHERE is_default),
         inserted AS (
             INSERT INTO user_groups (user_id, group_id)
             SELECT u.id, d.id FROM users u CROSS JOIN defaults d WHERE u.id = $id
             ON CONFLICT DO NOTHING
         )
         SELECT id FROM defaults",
        json!({ "id": user_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if !rows.is_empty() {
        return Ok(());
    }

    let group_id = Uuid::new_v4().to_string();
    state.db.query(
        "WITH g AS (
             INSERT INTO groups (id, name, description, is_default, created_at)
             SELECT $gid, u.name || '''s workspace', '', false, now() FROM users u WHERE u.id = $uid
             RETURNING id
         )
         INSERT INTO user_groups (user_id, group_id) SELECT $uid, id FROM g
         RETURNING group_id AS id",
        json!({ "uid": user_id, "gid": group_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(())
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
    let rows = state.db.query(
        "SELECT id, email, name, role, password_hash
         FROM users WHERE email = $email AND provider = 'local'",
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
    pub last_llm_provider_id: Option<String>,
    pub last_llm_model: Option<String>,
}

pub async fn me(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&state.config.jwt_secret, &jar)?;
    let rows = state.db.query(
        "SELECT last_project_id, last_llm_provider_id, last_llm_model
         FROM users WHERE id = $id",
        json!({ "id": claims.sub }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next();
    let field = |name: &str| row.as_ref().and_then(|r| r[name].as_str().map(String::from));
    Ok(Json(MeResponse {
        id: claims.sub,
        email: claims.email,
        name: claims.name,
        role: claims.role,
        last_project_id: field("last_project_id"),
        last_llm_provider_id: field("last_llm_provider_id"),
        last_llm_model: field("last_llm_model"),
    }))
}

#[derive(Deserialize, Default)]
pub struct UpdateMeBody {
    #[serde(default)]
    pub last_project_id: Option<String>,
    #[serde(default)]
    pub last_llm_provider_id: Option<String>,
    #[serde(default)]
    pub last_llm_model: Option<String>,
}

pub async fn update_me(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
    Json(body): Json<UpdateMeBody>,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&state.config.jwt_secret, &jar)?;

    state.db.execute(
        "UPDATE users SET
             last_project_id      = COALESCE($pid, last_project_id),
             last_llm_provider_id = COALESCE($provider_id, last_llm_provider_id),
             last_llm_model       = COALESCE($model, last_llm_model)
         WHERE id = $id",
        json!({
            "id": claims.sub,
            "pid": body.last_project_id,
            "provider_id": body.last_llm_provider_id,
            "model": body.last_llm_model,
        }),
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

    state.oauth_sessions.insert(oauth_state.clone(), OAuthSession {
        pkce_verifier: None,
        created_at: std::time::Instant::now(),
    });

    Ok((jar, Redirect::to(&url)))
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

    let session = state.oauth_sessions.remove(&oauth_state)
        .map(|(_, s)| s)
        .filter(|s| s.created_at.elapsed().as_secs() < SESSION_TTL_SECS)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "invalid OAuth state"))?;

    let access_token = exchange_google_code(&state.http, google, &code)
        .await
        .map_err(|_| err(StatusCode::BAD_GATEWAY, "failed to exchange OAuth code"))?;

    let google_user = fetch_google_user(&state.http, &access_token)
        .await
        .map_err(|_| err(StatusCode::BAD_GATEWAY, "failed to fetch Google user info"))?;

    let id = Uuid::new_v4().to_string();
    let now = harvest_db::now_rfc3339();

    let rows = insert_user(&state.db,
        "INSERT INTO users (id, google_id, email, name, provider, created_at, role)
         VALUES ($id, $google_id, $email, $name, 'google', $created_at,
                 CASE WHEN EXISTS (SELECT 1 FROM users) THEN 'regular' ELSE 'admin' END)
         ON CONFLICT (google_id) DO UPDATE SET email = EXCLUDED.email, name = EXCLUDED.name
         RETURNING id, email, name, role, (id = $id) AS is_new",
        json!({
            "google_id": google_user.id,
            "id": id,
            "email": google_user.email,
            "name": google_user.name,
            "created_at": now
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    let user = rows.into_iter().next().ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if user["is_new"].as_bool().unwrap_or(false) {
        assign_default_groups(&state, user["id"].as_str().unwrap_or_default()).await?;
    }
    let token = issue_token(&state.config.jwt_secret, &user)?;

    let _ = session; // session consumed above to validate state
    Ok((jar.add(make_token_cookie(token)), Redirect::to("/")))
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

    state.oauth_sessions.insert(oauth_state.clone(), OAuthSession {
        pkce_verifier: Some(pkce_verifier),
        created_at: std::time::Instant::now(),
    });

    Ok((jar, Redirect::to(&url)))
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

    let session = state.oauth_sessions.remove(&oauth_state)
        .map(|(_, s)| s)
        .filter(|s| s.created_at.elapsed().as_secs() < SESSION_TTL_SECS)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "invalid OAuth state"))?;

    let pkce_verifier = session.pkce_verifier;

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
    let now = harvest_db::now_rfc3339();

    let rows = insert_user(&state.db,
        "INSERT INTO users (id, email, name, provider, created_at, role, oidc_sub)
         VALUES ($id, $email, $name, 'oidc', $created_at,
                 CASE WHEN EXISTS (SELECT 1 FROM users) THEN 'regular' ELSE 'admin' END, $oidc_sub)
         ON CONFLICT (email) DO UPDATE SET name = EXCLUDED.name, oidc_sub = EXCLUDED.oidc_sub
         RETURNING id, email, name, role, (id = $id) AS is_new",
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
    if user["is_new"].as_bool().unwrap_or(false) {
        assign_default_groups(&state, user["id"].as_str().unwrap_or_default()).await?;
    }
    let token = issue_token(&state.config.jwt_secret, &user)?;

    Ok((jar.add(make_token_cookie(token)), Redirect::to("/")))
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

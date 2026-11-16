use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;

use super::super::handlers::CodeState;
use super::super::images::tail;
use super::super::{CodeEnv, WORKSPACE};
use super::clones::{self, Target};
use super::origin::PreviewOrigin;
use super::pass::Passes;
use super::site::ENTER_PATH;
use super::visit::{Identity, Visit};
use super::{runtime, Audience, Kind};
use crate::auth::jwt::{self, Claims};
use crate::auth::{cookie, token_from_headers};
use crate::lxd::containers::Command;

const VIEWER_COOKIE: &str = "harvest_viewer";
const RESPONSE_LIMIT: usize = 256 * 1024;
const EXEC_TIMEOUT: Duration = Duration::from_secs(120);

pub struct PreviewState {
    pub env: Option<CodeEnv>,
    pub jwt_secret: Arc<String>,
    pub passes: Arc<Passes>,
    pub origin: Option<PreviewOrigin>,
    pub http: reqwest::Client,
}

impl PreviewState {
    pub fn new(env: Option<CodeEnv>, jwt_secret: Arc<String>, passes: Arc<Passes>, origin: Option<PreviewOrigin>) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        Self { env, jwt_secret, passes, origin, http }
    }
}

fn failure(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn not_found() -> Response {
    failure(StatusCode::NOT_FOUND, "this preview does not exist or is not shared with you")
}

struct HarvestVisit {
    visit: Visit,
    new_cookie: Option<String>,
}

impl HarvestVisit {
    fn finish(&self, response: impl IntoResponse) -> Response {
        let mut response = response.into_response();
        if let Some(token) = &self.new_cookie {
            let cookie = format!("{VIEWER_COOKIE}={token}; Path=/previews; HttpOnly; SameSite=Lax; Max-Age=86400");
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                response.headers_mut().append(header::SET_COOKIE, value);
            }
        }
        response
    }

    async fn ready_container(&self) -> Result<String, Response> {
        match self.visit.target().await {
            Ok(Target::Ready(container)) => Ok(container),
            Ok(Target::Starting) => Err(failure(StatusCode::SERVICE_UNAVAILABLE, "your copy of the app is still starting")),
            Ok(Target::Failed(error)) => Err(failure(StatusCode::BAD_GATEWAY, error)),
            Err(e) => Err(failure(StatusCode::GONE, e.to_string())),
        }
    }
}

fn identity(state: &PreviewState, headers: &HeaderMap) -> (Identity, Option<String>) {
    let claims = token_from_headers(headers).and_then(|t| jwt::validate(&state.jwt_secret, &t).ok());
    if let Some(claims) = claims {
        return (Identity::User(claims.sub), None);
    }
    match cookie(headers, VIEWER_COOKIE).filter(|t| t.len() == 32) {
        Some(token) => (Identity::Anonymous(token), None),
        None => {
            let token = uuid::Uuid::new_v4().simple().to_string();
            (Identity::Anonymous(token.clone()), Some(token))
        }
    }
}

async fn visit(state: &PreviewState, id: &str, headers: &HeaderMap) -> Result<HarvestVisit, Response> {
    let env = state
        .env
        .as_ref()
        .ok_or_else(|| failure(StatusCode::SERVICE_UNAVAILABLE, "previews need LXD"))?;
    let (identity, new_cookie) = identity(state, headers);
    let visit = Visit::open(env, id, identity)
        .await
        .map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?
        .ok_or_else(not_found)?;
    Ok(HarvestVisit { visit, new_cookie })
}

pub async fn info(
    State(state): State<Arc<PreviewState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let harvest_visit = match visit(&state, &id, &headers).await {
        Ok(visit) => visit,
        Err(response) => return response,
    };
    let visit = &harvest_visit.visit;
    let (status, error) = match visit.target().await {
        Ok(Target::Ready(_)) => ("ready", None),
        Ok(Target::Starting) => ("starting", None),
        Ok(Target::Failed(error)) => ("failed", Some(error)),
        Err(_) => ("ended", None),
    };
    let preview = &visit.preview;
    harvest_visit.finish(Json(json!({
        "id": preview.id,
        "kind": preview.kind,
        "project": preview.project_name,
        "branch": preview.branch,
        "audience": preview.audience,
        "is_owner": visit.is_owner(),
        "status": status,
        "error": error,
        "web_enabled": state.origin.is_some(),
    })))
}

pub async fn enter(
    State(state): State<Arc<PreviewState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let harvest_visit = match visit(&state, &id, &headers).await {
        Ok(visit) => visit,
        Err(response) => return response,
    };
    let visit = &harvest_visit.visit;
    if visit.preview.kind != Kind::Web {
        return failure(StatusCode::BAD_REQUEST, "this preview is not a web app");
    }
    let Some(origin) = &state.origin else {
        return failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "web previews are turned off on this server; an administrator sets preview_domain under [code] to enable them",
        );
    };
    match state.passes.ticket(&id, &visit.viewer.key()) {
        Ok(ticket) => {
            let url = origin.url_for(&id, &format!("{ENTER_PATH}?ticket={ticket}"));
            harvest_visit.finish(Json(json!({ "url": url })))
        }
        Err(e) => failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

pub async fn reset(
    State(state): State<Arc<PreviewState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let harvest_visit = match visit(&state, &id, &headers).await {
        Ok(visit) => visit,
        Err(response) => return response,
    };
    let visit = &harvest_visit.visit;
    if visit.is_owner() {
        return failure(StatusCode::BAD_REQUEST, "your own preview runs in the session itself");
    }
    match clones::reset(&visit.env, &visit.preview, &visit.viewer).await {
        Ok(()) => harvest_visit.finish(StatusCode::NO_CONTENT),
        Err(e) => failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

pub async fn moved(Path(params): Path<HashMap<String, String>>) -> Redirect {
    let id = params.get("id").cloned().unwrap_or_default();
    Redirect::permanent(&format!("/#/preview/{id}"))
}

#[derive(Deserialize)]
pub struct ApiRequest {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
}

pub async fn send_request(
    State(state): State<Arc<PreviewState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ApiRequest>,
) -> Response {
    let harvest_visit = match visit(&state, &id, &headers).await {
        Ok(visit) => visit,
        Err(response) => return response,
    };
    let visit = &harvest_visit.visit;
    if visit.preview.kind != Kind::Api {
        return failure(StatusCode::BAD_REQUEST, "this preview is not an HTTP API");
    }
    let container = match harvest_visit.ready_container().await {
        Ok(container) => container,
        Err(response) => return harvest_visit.finish(response),
    };
    let address = match runtime::address(&visit.env, &container, &visit.preview).await {
        Ok(address) => address,
        Err(e) => return failure(StatusCode::BAD_GATEWAY, e.to_string()),
    };
    let Ok(method) = Method::from_bytes(request.method.trim().to_ascii_uppercase().as_bytes()) else {
        return failure(StatusCode::BAD_REQUEST, "unknown HTTP method");
    };
    let mut outgoing = HeaderMap::new();
    for (name, value) in &request.headers {
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value)) {
            outgoing.append(name, value);
        }
    }
    let path = request.path.trim().trim_start_matches('/');
    let started = Instant::now();
    let sent = state
        .http
        .request(method, format!("http://{address}/{path}"))
        .headers(outgoing)
        .body(request.body.unwrap_or_default())
        .send()
        .await;
    let upstream = match sent {
        Ok(upstream) => upstream,
        Err(e) => return harvest_visit.finish(failure(StatusCode::BAD_GATEWAY, format!("the API did not answer: {e}"))),
    };
    let status = upstream.status().as_u16();
    let response_headers: HashMap<String, String> = upstream
        .headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
        .collect();
    let bytes = upstream.bytes().await.unwrap_or_default();
    let truncated = bytes.len() > RESPONSE_LIMIT;
    let body = String::from_utf8_lossy(&bytes[..bytes.len().min(RESPONSE_LIMIT)]).into_owned();
    harvest_visit.finish(Json(json!({
        "status": status,
        "headers": response_headers,
        "body": body,
        "truncated": truncated,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    })))
}

#[derive(Deserialize)]
pub struct ExecRequest {
    pub command: String,
}

pub async fn exec(
    State(state): State<Arc<PreviewState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ExecRequest>,
) -> Response {
    let harvest_visit = match visit(&state, &id, &headers).await {
        Ok(visit) => visit,
        Err(response) => return response,
    };
    let visit = &harvest_visit.visit;
    if visit.preview.kind != Kind::Cli {
        return failure(StatusCode::BAD_REQUEST, "this preview is not a command-line tool");
    }
    let container = match harvest_visit.ready_container().await {
        Ok(container) => container,
        Err(response) => return harvest_visit.finish(response),
    };
    let command = Command::shell(&request.command).in_dir(WORKSPACE).timeout(EXEC_TIMEOUT);
    match visit.env.lxd.run(&container, command).await {
        Ok(output) => harvest_visit.finish(Json(json!({
            "exit_code": output.exit_code,
            "stdout": tail(&output.stdout, RESPONSE_LIMIT),
            "stderr": tail(&output.stderr, RESPONSE_LIMIT / 4),
        }))),
        Err(e) => harvest_visit.finish(failure(StatusCode::BAD_GATEWAY, e.to_string())),
    }
}

pub async fn session_preview(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(env) = &state.env else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "previews need LXD");
    };
    if let Err(response) = super::super::handlers::session_for(env, &user, &id).await {
        return response;
    }
    match super::for_session(&env.db, &id).await {
        Ok(preview) => Json(json!({ "preview": preview })).into_response(),
        Err(e) => failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct ShareBody {
    pub audience: String,
}

pub async fn share(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(id): Path<String>,
    Json(body): Json<ShareBody>,
) -> Response {
    let Some(env) = &state.env else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "previews need LXD");
    };
    if let Err(response) = super::super::handlers::session_for(env, &user, &id).await {
        return response;
    }
    let Some(audience) = Audience::parse(&body.audience) else {
        return failure(StatusCode::BAD_REQUEST, "choose owner, repository or link");
    };
    match super::set_audience(&env.db, &id, audience).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

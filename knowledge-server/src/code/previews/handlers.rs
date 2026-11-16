use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Redirect, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;

use super::super::handlers::CodeState;
use super::super::images::tail;
use super::super::{CodeEnv, WORKSPACE};
use super::clones::{self, Target};
use super::{base_path, may_view, runtime, Audience, Kind, Preview, Viewer};
use crate::access::repos::RepoAccess;
use crate::auth::jwt::{self, Claims};
use crate::auth::{cookie, token_from_headers};
use crate::lxd::containers::Command;

const VIEWER_COOKIE: &str = "harvest_viewer";
const RESPONSE_LIMIT: usize = 256 * 1024;
const EXEC_TIMEOUT: Duration = Duration::from_secs(120);
const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub struct PreviewState {
    pub env: Option<CodeEnv>,
    pub jwt_secret: Arc<String>,
    pub http: reqwest::Client,
}

impl PreviewState {
    pub fn new(env: Option<CodeEnv>, jwt_secret: Arc<String>) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        Self { env, jwt_secret, http }
    }
}

fn failure(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn not_found() -> Response {
    failure(StatusCode::NOT_FOUND, "this preview does not exist or is not shared with you")
}

struct Visit {
    env: CodeEnv,
    preview: Preview,
    viewer: Viewer,
    new_cookie: Option<String>,
}

impl Visit {
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

    async fn target(&self) -> Result<Target, Response> {
        clones::target_for(&self.env, &self.preview, &self.viewer)
            .await
            .map_err(|e| failure(StatusCode::GONE, e.to_string()))
    }

    async fn ready_container(&self) -> Result<String, Response> {
        match self.target().await? {
            Target::Ready(container) => Ok(container),
            Target::Starting => Err(failure(StatusCode::SERVICE_UNAVAILABLE, "your copy of the app is still starting")),
            Target::Failed(error) => Err(failure(StatusCode::BAD_GATEWAY, error)),
        }
    }
}

async fn visit(state: &PreviewState, id: &str, headers: &HeaderMap) -> Result<Visit, Response> {
    let env = state
        .env
        .clone()
        .ok_or_else(|| failure(StatusCode::SERVICE_UNAVAILABLE, "previews need LXD"))?;
    let preview = super::get(&env.db, id)
        .await
        .map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?
        .ok_or_else(not_found)?;
    let claims = token_from_headers(headers).and_then(|t| jwt::validate(&state.jwt_secret, &t).ok());
    let (viewer, new_cookie) = match claims {
        Some(claims) => {
            let repo_allowed = RepoAccess::new(&env.db)
                .scope_for(&claims.sub)
                .await
                .map(|scope| scope.allows(&preview.project_name))
                .unwrap_or(false);
            (Viewer::User { id: claims.sub, repo_allowed }, None)
        }
        None => match cookie(headers, VIEWER_COOKIE).filter(|t| t.len() == 32) {
            Some(token) => (Viewer::Anonymous(token), None),
            None => {
                let token = uuid::Uuid::new_v4().simple().to_string();
                (Viewer::Anonymous(token.clone()), Some(token))
            }
        },
    };
    if !may_view(&preview, &viewer) {
        return Err(not_found());
    }
    Ok(Visit { env, preview, viewer, new_cookie })
}

pub async fn info(
    State(state): State<Arc<PreviewState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let visit = match visit(&state, &id, &headers).await {
        Ok(visit) => visit,
        Err(response) => return response,
    };
    let (status, error) = match visit.target().await {
        Ok(Target::Ready(_)) => ("ready", None),
        Ok(Target::Starting) => ("starting", None),
        Ok(Target::Failed(error)) => ("failed", Some(error)),
        Err(_) => ("ended", None),
    };
    let preview = &visit.preview;
    visit.finish(Json(json!({
        "id": preview.id,
        "kind": preview.kind,
        "project": preview.project_name,
        "branch": preview.branch,
        "audience": preview.audience,
        "is_owner": visit.viewer.is_user(&preview.owner_id),
        "status": status,
        "error": error,
        "base_path": base_path(&preview.id),
    })))
}

pub async fn reset(
    State(state): State<Arc<PreviewState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let visit = match visit(&state, &id, &headers).await {
        Ok(visit) => visit,
        Err(response) => return response,
    };
    if visit.viewer.is_user(&visit.preview.owner_id) {
        return failure(StatusCode::BAD_REQUEST, "your own preview runs in the session itself");
    }
    match clones::reset(&visit.env, &visit.preview, &visit.viewer).await {
        Ok(()) => visit.finish(StatusCode::NO_CONTENT),
        Err(e) => failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

pub async fn web_root(Path(id): Path<String>) -> Redirect {
    Redirect::permanent(&base_path(&id))
}

pub async fn web(
    State(state): State<Arc<PreviewState>>,
    Path(params): Path<HashMap<String, String>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let id = params.get("id").cloned().unwrap_or_default();
    let path = params.get("path").cloned().unwrap_or_default();
    let visit = match visit(&state, &id, &headers).await {
        Ok(visit) => visit,
        Err(response) => return response,
    };
    if visit.preview.kind != Kind::Web {
        return failure(StatusCode::BAD_REQUEST, "this preview is not a web app");
    }
    let container = match visit.target().await {
        Ok(Target::Ready(container)) => container,
        Ok(Target::Starting) => return visit.finish(waiting_page()),
        Ok(Target::Failed(error)) => return visit.finish(failure(StatusCode::BAD_GATEWAY, error)),
        Err(response) => return response,
    };
    let address = match runtime::address(&visit.env, &container, &visit.preview).await {
        Ok(address) => address,
        Err(e) => return failure(StatusCode::BAD_GATEWAY, e.to_string()),
    };
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let target = format!("http://{address}/{path}{query}");
    let request = state
        .http
        .request(method, &target)
        .headers(forwarded_headers(&headers))
        .body(body);
    match request.send().await {
        Ok(upstream) => visit.finish(relay(upstream, &base_path(&id))),
        Err(e) => failure(StatusCode::BAD_GATEWAY, format!("the app did not answer: {e}")),
    }
}

fn waiting_page() -> Response {
    let html = "<!doctype html><meta charset=utf-8><meta http-equiv=refresh content=3>\
        <body style=\"font-family:sans-serif;color:#666;display:grid;place-items:center;height:90vh\">\
        Starting your copy of the app…</body>";
    (StatusCode::SERVICE_UNAVAILABLE, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
}

pub fn forwarded_headers(incoming: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in incoming {
        let lower = name.as_str();
        if HOP_BY_HOP.contains(&lower) || lower == "host" || lower == "content-length" {
            continue;
        }
        if name == header::COOKIE {
            if let Some(kept) = strip_harvest_cookies(value) {
                headers.append(header::COOKIE, kept);
            }
            continue;
        }
        headers.append(name.clone(), value.clone());
    }
    headers
}

fn strip_harvest_cookies(value: &HeaderValue) -> Option<HeaderValue> {
    let kept: Vec<&str> = value
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .filter(|c| !c.starts_with("token=") && !c.starts_with(&format!("{VIEWER_COOKIE}=")))
        .collect();
    if kept.is_empty() {
        return None;
    }
    HeaderValue::from_str(&kept.join("; ")).ok()
}

pub fn rebase_location(location: &str, base: &str) -> String {
    match location.strip_prefix('/') {
        Some(rest) if !location.starts_with("//") && !location.starts_with(base) => format!("{base}{rest}"),
        _ => location.to_string(),
    }
}

fn relay(upstream: reqwest::Response, base: &str) -> Response {
    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for (name, value) in upstream.headers() {
        if HOP_BY_HOP.contains(&name.as_str()) {
            continue;
        }
        if name == header::LOCATION {
            if let Some(rebased) = value
                .to_str()
                .ok()
                .and_then(|l| HeaderValue::from_str(&rebase_location(l, base)).ok())
            {
                builder = builder.header(name, rebased);
                continue;
            }
        }
        builder = builder.header(name, value);
    }
    builder
        .body(Body::from_stream(upstream.bytes_stream()))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
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
    let visit = match visit(&state, &id, &headers).await {
        Ok(visit) => visit,
        Err(response) => return response,
    };
    if visit.preview.kind != Kind::Api {
        return failure(StatusCode::BAD_REQUEST, "this preview is not an HTTP API");
    }
    let container = match visit.ready_container().await {
        Ok(container) => container,
        Err(response) => return visit.finish(response),
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
        Err(e) => return visit.finish(failure(StatusCode::BAD_GATEWAY, format!("the API did not answer: {e}"))),
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
    visit.finish(Json(json!({
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
    let visit = match visit(&state, &id, &headers).await {
        Ok(visit) => visit,
        Err(response) => return response,
    };
    if visit.preview.kind != Kind::Cli {
        return failure(StatusCode::BAD_REQUEST, "this preview is not a command-line tool");
    }
    let container = match visit.ready_container().await {
        Ok(container) => container,
        Err(response) => return visit.finish(response),
    };
    let command = Command::shell(&request.command).in_dir(WORKSPACE).timeout(EXEC_TIMEOUT);
    match visit.env.lxd.run(&container, command).await {
        Ok(output) => visit.finish(Json(json!({
            "exit_code": output.exit_code,
            "stdout": tail(&output.stdout, RESPONSE_LIMIT),
            "stderr": tail(&output.stderr, RESPONSE_LIMIT / 4),
        }))),
        Err(e) => visit.finish(failure(StatusCode::BAD_GATEWAY, e.to_string())),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harvest_credentials_never_reach_the_previewed_app() {
        let mut incoming = HeaderMap::new();
        incoming.insert(header::COOKIE, HeaderValue::from_static("token=secret; theme=dark; harvest_viewer=abc"));
        incoming.insert(header::HOST, HeaderValue::from_static("harvest.example"));
        incoming.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
        incoming.insert(header::ACCEPT, HeaderValue::from_static("text/html"));
        let forwarded = forwarded_headers(&incoming);
        assert_eq!(forwarded.get(header::COOKIE).unwrap(), "theme=dark");
        assert!(forwarded.get(header::HOST).is_none());
        assert!(forwarded.get(header::CONNECTION).is_none());
        assert_eq!(forwarded.get(header::ACCEPT).unwrap(), "text/html");
    }

    #[test]
    fn a_request_with_only_harvest_cookies_forwards_none() {
        let mut incoming = HeaderMap::new();
        incoming.insert(header::COOKIE, HeaderValue::from_static("token=secret"));
        assert!(forwarded_headers(&incoming).get(header::COOKIE).is_none());
    }

    #[test]
    fn absolute_redirects_stay_inside_the_preview() {
        let base = "/previews/p1/web/";
        assert_eq!(rebase_location("/login", base), "/previews/p1/web/login");
        assert_eq!(rebase_location("/previews/p1/web/x", base), "/previews/p1/web/x");
        assert_eq!(rebase_location("https://example.com/", base), "https://example.com/");
        assert_eq!(rebase_location("//cdn.example.com/a", base), "//cdn.example.com/a");
        assert_eq!(rebase_location("next", base), "next");
    }
}

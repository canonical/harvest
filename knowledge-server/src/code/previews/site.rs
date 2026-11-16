use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    body::Bytes,
    extract::{ws::WebSocketUpgrade, DefaultBodyLimit, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use serde::Deserialize;
use tower::ServiceExt as _;

use super::super::CodeEnv;
use super::clones::Target;
use super::framing;
use super::origin::PreviewOrigin;
use super::pass::{Passes, PASS_LIFETIME};
use super::proxy::{self, Visitor, PASS_COOKIE};
use super::visit::{Identity, Visit};
use super::{runtime, Kind};
use crate::auth::cookie;

pub const ENTER_PATH: &str = "/__harvest/enter";
const ADDRESS_TTL: Duration = Duration::from_secs(30);
const BODY_LIMIT: usize = 64 * 1024 * 1024;

pub struct Site {
    pub env: CodeEnv,
    pub origin: PreviewOrigin,
    pub passes: Arc<Passes>,
    pub harvest_url: String,
    harvest_origin: String,
    http: reqwest::Client,
    addresses: Mutex<HashMap<String, (String, Instant)>>,
}

impl Site {
    pub fn new(env: CodeEnv, origin: PreviewOrigin, passes: Arc<Passes>, harvest_url: String) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        Self {
            env,
            origin,
            passes,
            harvest_origin: framing::origin_of(&harvest_url),
            harvest_url,
            http,
            addresses: Mutex::new(HashMap::new()),
        }
    }

    fn cached_address(&self, container: &str) -> Option<String> {
        let addresses = self.addresses.lock().unwrap_or_else(|p| p.into_inner());
        addresses
            .get(container)
            .filter(|(_, at)| at.elapsed() < ADDRESS_TTL)
            .map(|(address, _)| address.clone())
    }

    fn remember_address(&self, container: &str, address: Option<&str>) {
        let mut addresses = self.addresses.lock().unwrap_or_else(|p| p.into_inner());
        match address {
            Some(address) => addresses.insert(container.to_string(), (address.to_string(), Instant::now())),
            None => addresses.remove(container),
        };
    }

    async fn address(&self, visit: &Visit, container: &str) -> anyhow::Result<String> {
        if let Some(address) = self.cached_address(container) {
            return Ok(address);
        }
        let address = runtime::address(&visit.env, container, &visit.preview).await?;
        self.remember_address(container, Some(&address));
        Ok(address)
    }

    fn harvest_link(&self, preview_id: &str) -> String {
        format!("{}/#/preview/{preview_id}", self.harvest_url.trim_end_matches('/'))
    }
}

pub fn router(site: Arc<Site>) -> Router {
    Router::new()
        .route(ENTER_PATH, get(enter))
        .fallback(serve)
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(site)
}

pub fn by_host(harvest: Router, previews: Router, origin: PreviewOrigin) -> Router {
    Router::new().fallback(move |request: Request| {
        let target = if origin.id_from_host(host_of(request.headers())).is_some() {
            previews.clone()
        } else {
            harvest.clone()
        };
        async move { target.oneshot(request).await.unwrap_or_else(|never: Infallible| match never {}) }
    })
}

fn host_of(headers: &HeaderMap) -> &str {
    headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or_default()
}

fn page(status: StatusCode, message: &str) -> Response {
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\">\
         <body style=\"font-family:Ubuntu,sans-serif;color:#333;display:grid;place-items:center;height:90vh;margin:0 16px\">\
         <p>{message}</p></body>"
    );
    (status, Html(html)).into_response()
}

fn waiting_page() -> Response {
    let mut response = page(StatusCode::SERVICE_UNAVAILABLE, "Starting your copy of the app…");
    response.headers_mut().insert(header::REFRESH, HeaderValue::from_static("3"));
    response
}

fn not_shared(site: &Site, preview_id: &str) -> Response {
    let link = site.harvest_link(preview_id);
    page(
        StatusCode::NOT_FOUND,
        &format!("This preview does not exist or is not shared with you. Open it from <a href=\"{link}\">Harvest</a>."),
    )
}

fn preview_id(site: &Site, headers: &HeaderMap) -> Option<String> {
    site.origin.id_from_host(host_of(headers))
}

#[derive(Deserialize)]
pub struct EnterQuery {
    ticket: String,
    to: Option<String>,
}

async fn enter(State(site): State<Arc<Site>>, headers: HeaderMap, Query(query): Query<EnterQuery>) -> Response {
    let Some(id) = preview_id(&site, &headers) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(identity) = site
        .passes
        .redeem_ticket(&query.ticket, &id)
        .ok()
        .and_then(|key| Identity::parse(&key))
    else {
        return not_shared(&site, &id);
    };
    let visit = match Visit::open(&site.env, &id, identity).await {
        Ok(Some(visit)) => visit,
        Ok(None) => return not_shared(&site, &id),
        Err(_) => return page(StatusCode::INTERNAL_SERVER_ERROR, "Harvest could not open this preview."),
    };
    let Ok(pass) = site.passes.pass(&id, &visit.viewer.key()) else {
        return page(StatusCode::INTERNAL_SERVER_ERROR, "Harvest could not open this preview.");
    };
    let mut response = Redirect::to(&landing_path(query.to.as_deref())).into_response();
    if let Ok(cookie) = HeaderValue::from_str(&pass_cookie(&pass, site.origin.is_secure())) {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    response
}

pub fn landing_path(to: Option<&str>) -> String {
    match to.map(str::trim) {
        Some(path) if path.starts_with('/') && !path.starts_with("//") && !path.contains('\\') => path.to_string(),
        _ => "/".to_string(),
    }
}

pub fn pass_cookie(pass: &str, secure: bool) -> String {
    let context = if secure { "SameSite=None; Secure; Partitioned" } else { "SameSite=Lax" };
    format!(
        "{PASS_COOKIE}={pass}; Path=/; HttpOnly; {context}; Max-Age={}",
        PASS_LIFETIME.as_secs()
    )
}

async fn serve(
    State(site): State<Arc<Site>>,
    ws: Option<WebSocketUpgrade>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(id) = preview_id(&site, &headers) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let identity = cookie(&headers, PASS_COOKIE)
        .and_then(|pass| site.passes.viewer_of_pass(&pass, &id).ok())
        .and_then(|key| Identity::parse(&key));
    let Some(identity) = identity else {
        return not_shared(&site, &id);
    };
    let visit = match Visit::open(&site.env, &id, identity).await {
        Ok(Some(visit)) => visit,
        Ok(None) => return not_shared(&site, &id),
        Err(_) => return page(StatusCode::INTERNAL_SERVER_ERROR, "Harvest could not open this preview."),
    };
    if visit.preview.kind != Kind::Web {
        return page(StatusCode::BAD_REQUEST, "This preview is not a web app.");
    }
    let container = match visit.target().await {
        Ok(Target::Ready(container)) => container,
        Ok(Target::Starting) => return waiting_page(),
        Ok(Target::Failed(error)) => return page(StatusCode::BAD_GATEWAY, &escape(&error)),
        Err(_) => return page(StatusCode::GONE, "This preview ended with its session."),
    };
    let address = match site.address(&visit, &container).await {
        Ok(address) => address,
        Err(e) => return page(StatusCode::BAD_GATEWAY, &format!("<pre>{}</pre>", escape(&e.to_string()))),
    };
    let host = host_of(&headers).to_string();
    let visitor = Visitor { host: &host, scheme: if site.origin.is_secure() { "https" } else { "http" } };
    if let Some(ws) = ws {
        return proxy::tunnel(ws, &address, &uri, &headers, &visitor).await;
    }
    let forwarded = proxy::forwarded_headers(&headers, &visitor);
    match proxy::forward(&site.http, &address, method, &uri, forwarded, body).await {
        Ok(mut response) => {
            framing::allow_framing_by(response.headers_mut(), &site.harvest_origin);
            response
        }
        Err(e) => {
            site.remember_address(&container, None);
            page(StatusCode::BAD_GATEWAY, &format!("The app did not answer: {}", escape(&e.to_string())))
        }
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pass_cookie_is_scoped_to_the_preview_host() {
        let cookie = pass_cookie("abc", false);
        assert_eq!(cookie, "harvest_preview=abc; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400");
        assert!(!cookie.contains("Domain="));
    }

    #[test]
    fn over_https_the_pass_also_works_inside_harvest_frames() {
        assert_eq!(
            pass_cookie("abc", true),
            "harvest_preview=abc; Path=/; HttpOnly; SameSite=None; Secure; Partitioned; Max-Age=86400"
        );
    }

    #[test]
    fn entering_only_lands_on_paths_of_the_preview() {
        assert_eq!(landing_path(Some("/admin?x=1")), "/admin?x=1");
        assert_eq!(landing_path(None), "/");
        for outside in ["//evil.example", "https://evil.example", "admin", "/\\evil"] {
            assert_eq!(landing_path(Some(outside)), "/", "{outside}");
        }
    }

    #[test]
    fn error_text_is_escaped_in_pages() {
        assert_eq!(escape("<script>&"), "&lt;script&gt;&amp;");
    }
}

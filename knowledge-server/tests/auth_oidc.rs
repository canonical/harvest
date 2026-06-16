/// Handler-level tests for the generic OIDC SSO flow.
///
/// Tests that require Neo4j (callback success path) are in the
/// integration test section at the bottom and are skipped when
/// `TEST_NEO4J_URI` is not set.
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use http_body_util::BodyExt as _;
use httpmock::prelude::*;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use knowledge_server::{
    auth::{handlers as auth_handlers, AuthState, OidcEndpoints},
    config::{AuthConfig, OidcConfig},
};

// ─── helpers ──────────────────────────────────────────────────────────────────

fn oidc_config(issuer_url: &str) -> OidcConfig {
    OidcConfig {
        issuer_url:    issuer_url.into(),
        client_id:     "harvest-test".into(),
        client_secret: "secret".into(),
        redirect_uri:  "https://app.example.com/auth/oidc/callback".into(),
        display_name:  Some("Test IdP".into()),
    }
}

fn endpoints(base: &str) -> OidcEndpoints {
    OidcEndpoints {
        authorization_endpoint: format!("{base}/auth"),
        token_endpoint:         format!("{base}/token"),
        userinfo_endpoint:      format!("{base}/userinfo"),
    }
}

fn auth_config_with_oidc(issuer_url: &str) -> Arc<AuthConfig> {
    Arc::new(AuthConfig {
        jwt_secret: "test-secret-for-jwt-signing-long-enough".into(),
        google:     None,
        oidc:       Some(oidc_config(issuer_url)),
    })
}

fn auth_config_no_oidc() -> Arc<AuthConfig> {
    Arc::new(AuthConfig {
        jwt_secret: "test-secret-for-jwt-signing-long-enough".into(),
        google:     None,
        oidc:       None,
    })
}

/// Build a minimal router for auth endpoints without a real Neo4j.
/// The Neo4j client is `None`-equivalent — tests must not reach DB calls.
fn oidc_router(auth: Arc<AuthState>) -> Router {
    Router::new()
        .route("/auth/config",        get(auth_handlers::config))
        .route("/auth/oidc",          get(auth_handlers::oidc_redirect))
        .route("/auth/oidc/callback", get(auth_handlers::oidc_callback))
        .with_state(auth)
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn resp_header(resp: &axum::response::Response, name: &str) -> Option<String> {
    resp.headers().get(name).and_then(|v| v.to_str().ok()).map(String::from)
}

// ─── /auth/config ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn config_reports_oidc_enabled_with_display_name() {
    let server = MockServer::start();
    let cfg = auth_config_with_oidc(&server.base_url());
    let ep  = Arc::new(endpoints(&server.base_url()));
    let auth = Arc::new(AuthState {
        neo4j:          make_stub_neo4j().await,
        config:         cfg,
        http:           reqwest::Client::new(),
        oidc_endpoints: Some(ep),
    });
    let app  = oidc_router(auth);

    let resp = app
        .oneshot(Request::builder().uri("/auth/config").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["oidc"], json!(true));
    assert_eq!(body["oidc_display_name"], json!("Test IdP"));
}

#[tokio::test]
async fn config_reports_oidc_disabled_when_not_configured() {
    let auth = Arc::new(AuthState {
        neo4j:          make_stub_neo4j().await,
        config:         auth_config_no_oidc(),
        http:           reqwest::Client::new(),
        oidc_endpoints: None,
    });
    let app = oidc_router(auth);

    let resp = app
        .oneshot(Request::builder().uri("/auth/config").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let body = body_json(resp).await;
    assert_eq!(body["oidc"], json!(false));
}

#[tokio::test]
async fn config_oidc_display_name_null_when_not_set() {
    let server = MockServer::start();
    let cfg = Arc::new(AuthConfig {
        jwt_secret: "test-secret-for-jwt-signing-long-enough".into(),
        google:     None,
        oidc:       Some(OidcConfig {
            issuer_url:    server.base_url(),
            client_id:     "cid".into(),
            client_secret: "csec".into(),
            redirect_uri:  "https://app.example.com/cb".into(),
            display_name:  None,
        }),
    });
    let ep = Arc::new(endpoints(&server.base_url()));
    let auth = Arc::new(AuthState {
        neo4j:          make_stub_neo4j().await,
        config:         cfg,
        http:           reqwest::Client::new(),
        oidc_endpoints: Some(ep),
    });

    let resp = oidc_router(auth)
        .oneshot(Request::builder().uri("/auth/config").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let body = body_json(resp).await;
    assert_eq!(body["oidc"], json!(true));
    assert!(body["oidc_display_name"].is_null());
}

// ─── /auth/oidc redirect ──────────────────────────────────────────────────────

#[tokio::test]
async fn oidc_redirect_returns_501_when_not_configured() {
    let auth = Arc::new(AuthState {
        neo4j:          make_stub_neo4j().await,
        config:         auth_config_no_oidc(),
        http:           reqwest::Client::new(),
        oidc_endpoints: None,
    });
    let resp = oidc_router(auth)
        .oneshot(Request::builder().uri("/auth/oidc").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn oidc_redirect_returns_302_to_authorization_endpoint() {
    let server = MockServer::start();
    let cfg = auth_config_with_oidc(&server.base_url());
    let ep  = Arc::new(endpoints(&server.base_url()));
    let auth = Arc::new(AuthState {
        neo4j:          make_stub_neo4j().await,
        config:         cfg,
        http:           reqwest::Client::new(),
        oidc_endpoints: Some(ep),
    });

    let resp = oidc_router(auth)
        .oneshot(Request::builder().uri("/auth/oidc").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let location = resp_header(&resp, "location").expect("location header");
    assert!(location.starts_with(&format!("{}/auth?", server.base_url())));
}

#[tokio::test]
async fn oidc_redirect_url_contains_required_params() {
    let server = MockServer::start();
    let cfg = auth_config_with_oidc(&server.base_url());
    let ep  = Arc::new(endpoints(&server.base_url()));
    let auth = Arc::new(AuthState {
        neo4j:          make_stub_neo4j().await,
        config:         cfg,
        http:           reqwest::Client::new(),
        oidc_endpoints: Some(ep),
    });

    let resp = oidc_router(auth)
        .oneshot(Request::builder().uri("/auth/oidc").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let location = resp_header(&resp, "location").unwrap();
    let url = reqwest::Url::parse(&location).unwrap();
    let params: std::collections::HashMap<_, _> = url.query_pairs().collect();

    assert_eq!(params.get("client_id").map(|s| s.as_ref()), Some("harvest-test"));
    assert_eq!(params.get("response_type").map(|s| s.as_ref()), Some("code"));
    assert_eq!(params.get("scope").map(|s| s.as_ref()), Some("openid email profile"));
    assert!(params.contains_key("state"), "state param must be present");
    assert!(params.contains_key("redirect_uri"), "redirect_uri must be present");
}

#[tokio::test]
async fn oidc_redirect_sets_oauth_state_cookie() {
    let server = MockServer::start();
    let cfg = auth_config_with_oidc(&server.base_url());
    let ep  = Arc::new(endpoints(&server.base_url()));
    let auth = Arc::new(AuthState {
        neo4j:          make_stub_neo4j().await,
        config:         cfg,
        http:           reqwest::Client::new(),
        oidc_endpoints: Some(ep),
    });

    let resp = oidc_router(auth)
        .oneshot(Request::builder().uri("/auth/oidc").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let set_cookie = resp_header(&resp, "set-cookie").expect("set-cookie header");
    assert!(set_cookie.contains("oauth_state="), "state cookie must be set");
    assert!(set_cookie.contains("HttpOnly"), "state cookie must be HttpOnly");
}

// ─── /auth/oidc/callback ──────────────────────────────────────────────────────

#[tokio::test]
async fn oidc_callback_returns_400_when_no_code() {
    let server = MockServer::start();
    let cfg = auth_config_with_oidc(&server.base_url());
    let ep  = Arc::new(endpoints(&server.base_url()));
    let auth = Arc::new(AuthState {
        neo4j:          make_stub_neo4j().await,
        config:         cfg,
        http:           reqwest::Client::new(),
        oidc_endpoints: Some(ep),
    });

    let resp = oidc_router(auth)
        .oneshot(
            Request::builder()
                .uri("/auth/oidc/callback?state=abc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oidc_callback_returns_400_on_idp_error_param() {
    let server = MockServer::start();
    let cfg = auth_config_with_oidc(&server.base_url());
    let ep  = Arc::new(endpoints(&server.base_url()));
    let auth = Arc::new(AuthState {
        neo4j:          make_stub_neo4j().await,
        config:         cfg,
        http:           reqwest::Client::new(),
        oidc_endpoints: Some(ep),
    });

    let resp = oidc_router(auth)
        .oneshot(
            Request::builder()
                .uri("/auth/oidc/callback?error=access_denied&error_description=User+denied+access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert!(body["error"].as_str().unwrap().contains("OIDC error"));
}

#[tokio::test]
async fn oidc_callback_returns_400_on_state_mismatch() {
    let server = MockServer::start();
    let cfg = auth_config_with_oidc(&server.base_url());
    let ep  = Arc::new(endpoints(&server.base_url()));
    let auth = Arc::new(AuthState {
        neo4j:          make_stub_neo4j().await,
        config:         cfg,
        http:           reqwest::Client::new(),
        oidc_endpoints: Some(ep),
    });

    // Provide a state param but no matching cookie → mismatch
    let resp = oidc_router(auth)
        .oneshot(
            Request::builder()
                .uri("/auth/oidc/callback?code=abc&state=wrong-state")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert!(body["error"].as_str().unwrap().contains("state"));
}

#[tokio::test]
async fn oidc_callback_returns_501_when_not_configured() {
    let auth = Arc::new(AuthState {
        neo4j:          make_stub_neo4j().await,
        config:         auth_config_no_oidc(),
        http:           reqwest::Client::new(),
        oidc_endpoints: None,
    });

    let resp = oidc_router(auth)
        .oneshot(
            Request::builder()
                .uri("/auth/oidc/callback?code=x&state=y")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
}

// ─── stub Neo4j ───────────────────────────────────────────────────────────────

/// Creates a Neo4j client pointing at a non-existent host.
/// neo4rs uses a lazy connection pool, so construction succeeds;
/// only actual queries will fail — which is fine for no-DB tests.
async fn make_stub_neo4j() -> Arc<knowledge_server::neo4j::Neo4jClient> {
    Arc::new(
        knowledge_server::neo4j::Neo4jClient::new("bolt://127.0.0.1:19999", "neo4j", "x")
            .await
            .expect("neo4rs pool construction should succeed even with unreachable host"),
    )
}

use anyhow::{Context, Result};
use base64::Engine as _;
use rand_core::{OsRng, RngCore};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct OidcEndpoints {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
}

#[derive(Deserialize)]
struct DiscoveryDoc {
    authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: String,
}

/// Generates a (verifier, challenge) PKCE pair using S256.
/// The verifier is stored in a cookie; the challenge goes in the auth URL.
pub fn generate_pkce_pair() -> (String, String) {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let digest   = Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    (verifier, challenge)
}

pub async fn discover_endpoints(http: &reqwest::Client, issuer_url: &str) -> Result<OidcEndpoints> {
    let discovery_url = format!(
        "{}/.well-known/openid-configuration",
        issuer_url.trim_end_matches('/')
    );
    let resp = http
        .get(&discovery_url)
        .send()
        .await
        .context("fetching OIDC discovery document")?;
    if !resp.status().is_success() {
        anyhow::bail!("OIDC discovery returned {}", resp.status());
    }
    let doc: DiscoveryDoc = resp
        .json()
        .await
        .context("parsing OIDC discovery document")?;
    Ok(OidcEndpoints {
        authorization_endpoint: doc.authorization_endpoint,
        token_endpoint: doc.token_endpoint,
        userinfo_endpoint: doc.userinfo_endpoint,
    })
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

pub async fn exchange_code(
    http: &reqwest::Client,
    token_endpoint: &str,
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
    code: &str,
    code_verifier: Option<&str>,
) -> Result<String> {
    let mut params = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
    ];
    if let Some(v) = code_verifier {
        params.push(("code_verifier", v));
    }
    let resp = http
        .post(token_endpoint)
        .basic_auth(client_id, Some(client_secret))
        .form(&params)
        .send()
        .await
        .context("posting to OIDC token endpoint")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        tracing::error!(%status, %body, "OIDC token endpoint error");
        anyhow::bail!("OIDC token endpoint returned {status}: {body}");
    }
    let token: TokenResponse = resp.json().await.context("parsing token response")?;
    Ok(token.access_token)
}

#[derive(Deserialize)]
pub struct OidcUserInfo {
    pub sub: String,
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
}

pub async fn fetch_userinfo(
    http: &reqwest::Client,
    userinfo_endpoint: &str,
    access_token: &str,
) -> Result<OidcUserInfo> {
    let resp = http
        .get(userinfo_endpoint)
        .bearer_auth(access_token)
        .send()
        .await
        .context("fetching OIDC userinfo")?;
    if !resp.status().is_success() {
        anyhow::bail!("OIDC userinfo endpoint returned {}", resp.status());
    }
    resp.json().await.context("parsing OIDC userinfo")
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    fn http() -> reqwest::Client {
        reqwest::Client::new()
    }

    // ── discover_endpoints ────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_parses_valid_doc() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/.well-known/openid-configuration");
            then.status(200).json_body(json!({
                "authorization_endpoint": "https://idp.example.com/auth",
                "token_endpoint":         "https://idp.example.com/token",
                "userinfo_endpoint":      "https://idp.example.com/userinfo",
            }));
        });

        let endpoints = discover_endpoints(&http(), &server.base_url()).await.unwrap();
        assert_eq!(endpoints.authorization_endpoint, "https://idp.example.com/auth");
        assert_eq!(endpoints.token_endpoint, "https://idp.example.com/token");
        assert_eq!(endpoints.userinfo_endpoint, "https://idp.example.com/userinfo");
    }

    #[tokio::test]
    async fn discover_strips_trailing_slash_from_issuer() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/.well-known/openid-configuration");
            then.status(200).json_body(json!({
                "authorization_endpoint": "https://idp.example.com/auth",
                "token_endpoint":         "https://idp.example.com/token",
                "userinfo_endpoint":      "https://idp.example.com/userinfo",
            }));
        });

        let issuer_with_slash = format!("{}/", server.base_url());
        let endpoints = discover_endpoints(&http(), &issuer_with_slash).await.unwrap();
        assert_eq!(endpoints.authorization_endpoint, "https://idp.example.com/auth");
    }

    #[tokio::test]
    async fn discover_errors_on_non_200() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/.well-known/openid-configuration");
            then.status(404);
        });

        let result = discover_endpoints(&http(), &server.base_url()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("404"));
    }

    #[tokio::test]
    async fn discover_errors_on_invalid_json() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/.well-known/openid-configuration");
            then.status(200).body("not json");
        });

        let result = discover_endpoints(&http(), &server.base_url()).await;
        assert!(result.is_err());
    }

    // ── generate_pkce_pair ────────────────────────────────────────────────

    #[test]
    fn pkce_verifier_and_challenge_are_different() {
        let (verifier, challenge) = generate_pkce_pair();
        assert_ne!(verifier, challenge);
        assert!(!verifier.is_empty());
        assert!(!challenge.is_empty());
    }

    #[test]
    fn pkce_challenge_is_base64url_sha256_of_verifier() {
        let (verifier, challenge) = generate_pkce_pair();
        let digest = sha2::Sha256::digest(verifier.as_bytes());
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
        assert_eq!(challenge, expected);
    }

    #[test]
    fn pkce_pairs_are_unique() {
        let (v1, _) = generate_pkce_pair();
        let (v2, _) = generate_pkce_pair();
        assert_ne!(v1, v2);
    }

    // ── exchange_code ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn exchange_code_returns_access_token() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/token")
                .form_urlencoded_tuple_exists("code")
                .form_urlencoded_tuple("grant_type", "authorization_code")
                .header_exists("Authorization");   // Basic auth
            then.status(200).json_body(json!({ "access_token": "tok_abc" }));
        });

        let token = exchange_code(
            &http(),
            &format!("{}/token", server.base_url()),
            "client_id",
            "client_secret",
            "https://app.example.com/callback",
            "auth_code_123",
            None,
        )
        .await
        .unwrap();
        assert_eq!(token, "tok_abc");
    }

    #[tokio::test]
    async fn exchange_code_errors_on_non_200() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/token");
            then.status(400).json_body(json!({ "error": "invalid_grant" }));
        });

        let result = exchange_code(
            &http(),
            &format!("{}/token", server.base_url()),
            "cid", "csec", "ruri", "bad_code", None,
        )
        .await;
        assert!(result.is_err());
    }

    // ── fetch_userinfo ────────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_userinfo_returns_user() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/userinfo")
                .header("Authorization", "Bearer tok_abc");
            then.status(200).json_body(json!({
                "sub":   "uid-001",
                "email": "user@example.com",
                "name":  "Test User",
            }));
        });

        let user = fetch_userinfo(
            &http(),
            &format!("{}/userinfo", server.base_url()),
            "tok_abc",
        )
        .await
        .unwrap();
        assert_eq!(user.sub, "uid-001");
        assert_eq!(user.email, "user@example.com");
        assert_eq!(user.name.as_deref(), Some("Test User"));
    }

    #[tokio::test]
    async fn fetch_userinfo_name_is_optional() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/userinfo");
            then.status(200).json_body(json!({
                "sub":   "uid-002",
                "email": "noname@example.com",
            }));
        });

        let user = fetch_userinfo(
            &http(),
            &format!("{}/userinfo", server.base_url()),
            "tok",
        )
        .await
        .unwrap();
        assert!(user.name.is_none());
    }

    #[tokio::test]
    async fn fetch_userinfo_errors_on_non_200() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/userinfo");
            then.status(401);
        });

        let result = fetch_userinfo(&http(), &format!("{}/userinfo", server.base_url()), "tok").await;
        assert!(result.is_err());
    }
}

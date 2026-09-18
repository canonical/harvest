use anyhow::{anyhow, Result};
use reqwest::header::{HeaderValue, AUTHORIZATION, SET_COOKIE};
use serde::{de::DeserializeOwned, Serialize};
use std::time::Duration;

#[derive(Clone)]
pub struct Client {
    base_url: String,
    http: reqwest::Client,
    token: String,
}

impl Client {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
            token: token.into(),
        })
    }

    pub fn url(&self, path: &str) -> String {
        if path.starts_with('/') {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}/{}", self.base_url, path)
        }
    }

    fn auth_header(&self) -> HeaderValue {
        HeaderValue::from_str(&format!("Bearer {}", self.token)).expect("token must be a valid header value")
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self
            .http
            .get(self.url(path))
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .await?;
        Self::json_or_err("GET", path, resp).await
    }

    pub async fn post_json<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let resp = self
            .http
            .post(self.url(path))
            .header(AUTHORIZATION, self.auth_header())
            .json(body)
            .send()
            .await?;
        Self::json_or_err("POST", path, resp).await
    }

    pub async fn put_json<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let resp = self
            .http
            .put(self.url(path))
            .header(AUTHORIZATION, self.auth_header())
            .json(body)
            .send()
            .await?;
        Self::json_or_err("PUT", path, resp).await
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let resp = self
            .http
            .delete(self.url(path))
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("DELETE {path} -> {status}: {body}"));
        }
        Ok(())
    }

    async fn json_or_err<T: DeserializeOwned>(method: &str, path: &str, resp: reqwest::Response) -> Result<T> {
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("{method} {path} -> {status}: {body}"));
        }
        let text = resp.text().await?;
        serde_json::from_str(&text).map_err(|e| anyhow!("{method} {path} -> failed to parse response as JSON: {e}; body: {text}"))
    }
}

fn extract_token_cookie(set_cookie: &str) -> Option<String> {
    set_cookie
        .split(';')
        .next()
        .and_then(|kv| kv.trim().strip_prefix("token="))
        .map(str::to_string)
}

pub async fn login(base_url: &str, email: &str, password: &str) -> Result<String> {
    let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let base_url = base_url.trim_end_matches('/');
    let resp = http
        .post(format!("{base_url}/auth/login"))
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("login failed: {status}: {body}"));
    }
    let cookie = resp
        .headers()
        .get(SET_COOKIE)
        .ok_or_else(|| anyhow!("login response had no Set-Cookie header"))?
        .to_str()
        .map_err(|e| anyhow!("login response Set-Cookie header was not valid text: {e}"))?
        .to_string();
    extract_token_cookie(&cookie).ok_or_else(|| anyhow!("login response Set-Cookie header had no token value"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    #[test]
    fn url_joins_leading_slash_path() {
        let client = Client::new("http://localhost:8080", "tok").unwrap();
        assert_eq!(client.url("/projects"), "http://localhost:8080/projects");
    }

    #[test]
    fn url_joins_bare_path() {
        let client = Client::new("http://localhost:8080", "tok").unwrap();
        assert_eq!(client.url("projects"), "http://localhost:8080/projects");
    }

    #[test]
    fn new_trims_trailing_slash_from_base_url() {
        let client = Client::new("http://localhost:8080/", "tok").unwrap();
        assert_eq!(client.url("/projects"), "http://localhost:8080/projects");
    }

    #[test]
    fn extract_token_cookie_reads_value_before_first_semicolon() {
        let cookie = "token=abc.def.ghi; HttpOnly; SameSite=Lax; Path=/";
        assert_eq!(extract_token_cookie(cookie), Some("abc.def.ghi".to_string()));
    }

    #[test]
    fn extract_token_cookie_returns_none_when_no_token_key() {
        let cookie = "session=abc; HttpOnly";
        assert_eq!(extract_token_cookie(cookie), None);
    }

    #[tokio::test]
    async fn get_json_sends_bearer_token_and_parses_body() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/projects").header("authorization", "Bearer tok123");
            then.status(200).json_body(json!([{"id": "p1"}]));
        });
        let client = Client::new(server.base_url(), "tok123").unwrap();
        let projects: Vec<serde_json::Value> = client.get_json("/projects").await.unwrap();
        mock.assert();
        assert_eq!(projects[0]["id"], "p1");
    }

    #[tokio::test]
    async fn post_json_sends_body_and_parses_response() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/projects").json_body(json!({"name": "n"}));
            then.status(201).json_body(json!({"id": "p1"}));
        });
        let client = Client::new(server.base_url(), "tok").unwrap();
        let created: serde_json::Value = client.post_json("/projects", &json!({"name": "n"})).await.unwrap();
        mock.assert();
        assert_eq!(created["id"], "p1");
    }

    #[tokio::test]
    async fn non_success_status_returns_error_with_body() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/projects/missing");
            then.status(404).json_body(json!({"error": "not found"}));
        });
        let client = Client::new(server.base_url(), "tok").unwrap();
        let err = client.get_json::<serde_json::Value>("/projects/missing").await.unwrap_err();
        assert!(err.to_string().contains("404"));
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn plain_text_error_body_is_preserved_in_error_message() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/projects/p1/query");
            then.status(500).body("agent crashed unexpectedly");
        });
        let client = Client::new(server.base_url(), "tok").unwrap();
        let err = client.post_json::<_, serde_json::Value>("/projects/p1/query", &json!({})).await.unwrap_err();
        assert!(err.to_string().contains("agent crashed unexpectedly"));
    }

    #[tokio::test]
    async fn delete_succeeds_on_204() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(DELETE).path("/artifacts/a1");
            then.status(204);
        });
        let client = Client::new(server.base_url(), "tok").unwrap();
        client.delete("/artifacts/a1").await.unwrap();
    }

    #[tokio::test]
    async fn login_extracts_token_from_set_cookie_header() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/auth/login").json_body(json!({"email": "a@b.com", "password": "secret"}));
            then.status(200)
                .header("set-cookie", "token=xyz.abc.123; HttpOnly; SameSite=Lax; Path=/")
                .json_body(json!({"ok": true}));
        });
        let token = login(&server.base_url(), "a@b.com", "secret").await.unwrap();
        assert_eq!(token, "xyz.abc.123");
    }

    #[tokio::test]
    async fn login_fails_with_invalid_credentials_message() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/auth/login");
            then.status(401).json_body(json!({"error": "invalid credentials"}));
        });
        let err = login(&server.base_url(), "a@b.com", "wrong").await.unwrap_err();
        assert!(err.to_string().contains("invalid credentials"));
    }

    #[tokio::test]
    async fn login_fails_when_no_set_cookie_header_present() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/auth/login");
            then.status(200).json_body(json!({"ok": true}));
        });
        let err = login(&server.base_url(), "a@b.com", "secret").await.unwrap_err();
        assert!(err.to_string().contains("Set-Cookie"));
    }
}

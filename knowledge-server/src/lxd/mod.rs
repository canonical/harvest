pub mod identity;

use anyhow::{bail, Context, Result};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;

use crate::config::LxdConfig;
use crate::neo4j::Neo4jClient;

const DEFAULT_OPERATION_WAIT_SECS: u64 = 120;
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const IDENTITY_NAME: &str = "harvest";

fn wants_manual_cert(cfg: &LxdConfig) -> bool {
    cfg.client_cert.is_some() && cfg.client_key.is_some()
}

pub async fn resolve_client(cfg: &LxdConfig, neo4j: &Neo4jClient) -> Result<Option<LxdClient>> {
    if wants_manual_cert(cfg) {
        return Ok(Some(LxdClient::new(cfg)?));
    }

    let mut ident = identity::load_or_generate(neo4j).await?;

    if !ident.trusted {
        let Some(token) = &cfg.trust_token else {
            tracing::warn!(
                "LXD is configured but has no trusted client identity yet; set lxd.trust_token \
                 (from `lxc config trust add --name harvest`) to enable LXD-managed agents"
            );
            return Ok(None);
        };

        match identity::join_with_token(
            &cfg.endpoint, &ident, IDENTITY_NAME, token, cfg.ca_cert.as_deref(), cfg.insecure,
        ).await {
            Ok(()) => {
                identity::mark_trusted(neo4j).await?;
                ident.trusted = true;
                tracing::info!("LXD client identity successfully joined via trust token");
            }
            Err(e) => {
                tracing::warn!(error = %e, "LXD trust-token join failed; LXD-managed agents disabled until a valid token is supplied");
                return Ok(None);
            }
        }
    }

    Ok(Some(LxdClient::from_identity(&ident.client_cert, &ident.client_key, cfg)?))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Tiny,
    Small,
    Medium,
    Large,
    XLarge,
}

impl Flavor {
    pub fn all() -> &'static [Flavor] {
        &[Flavor::Tiny, Flavor::Small, Flavor::Medium, Flavor::Large, Flavor::XLarge]
    }

    pub fn id(&self) -> &'static str {
        match self {
            Flavor::Tiny => "tiny",
            Flavor::Small => "small",
            Flavor::Medium => "medium",
            Flavor::Large => "large",
            Flavor::XLarge => "extra-large",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Flavor::Tiny => "Tiny",
            Flavor::Small => "Small",
            Flavor::Medium => "Medium",
            Flavor::Large => "Large",
            Flavor::XLarge => "Extra Large",
        }
    }

    pub fn cpu(&self) -> u32 {
        match self {
            Flavor::Tiny | Flavor::Small => 1,
            Flavor::Medium => 2,
            Flavor::Large => 4,
            Flavor::XLarge => 8,
        }
    }

    pub fn memory_mib(&self) -> u32 {
        match self {
            Flavor::Tiny => 512,
            Flavor::Small => 1024,
            Flavor::Medium => 2048,
            Flavor::Large => 4096,
            Flavor::XLarge => 8192,
        }
    }

    pub fn from_id(id: &str) -> Option<Flavor> {
        Flavor::all().iter().copied().find(|f| f.id() == id)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id":         self.id(),
            "label":      self.label(),
            "cpu":        self.cpu(),
            "memory_mib": self.memory_mib(),
        })
    }
}

pub struct CreateInstance {
    pub name:        String,
    pub network:     String,
    pub description: String,
    pub flavor:      Flavor,
}

pub struct LxdClient {
    http:         Client,
    base_url:     String,
    project:      String,
    image_alias:  String,
    image_server: String,
    profile:      String,
}

pub(crate) fn build_http_client(
    cert_pem: &str,
    key_pem:  &str,
    ca_cert:  Option<&str>,
    insecure: bool,
) -> Result<Client> {
    let pem = format!("{cert_pem}\n{key_pem}");
    let identity = reqwest::Identity::from_pem(pem.as_bytes())
        .context("parsing LXD client certificate/key")?;

    let mut builder = Client::builder()
        .use_rustls_tls()
        .identity(identity)
        .timeout(Duration::from_secs(DEFAULT_OPERATION_WAIT_SECS + 60));

    if let Some(ca) = ca_cert {
        let cert = reqwest::Certificate::from_pem(ca.as_bytes())
            .context("parsing LXD CA certificate")?;
        builder = builder.add_root_certificate(cert);
    }
    if insecure {
        builder = builder.danger_accept_invalid_certs(true);
    }

    builder.build().context("building LXD HTTP client")
}

impl LxdClient {
    pub fn new(cfg: &LxdConfig) -> Result<Self> {
        let cert = cfg.client_cert.as_deref()
            .ok_or_else(|| anyhow::anyhow!("LxdClient::new requires client_cert to be set"))?;
        let key = cfg.client_key.as_deref()
            .ok_or_else(|| anyhow::anyhow!("LxdClient::new requires client_key to be set"))?;
        Self::from_identity(cert, key, cfg)
    }

    pub fn from_identity(cert_pem: &str, key_pem: &str, cfg: &LxdConfig) -> Result<Self> {
        let http = build_http_client(cert_pem, key_pem, cfg.ca_cert.as_deref(), cfg.insecure)?;

        Ok(Self {
            http,
            base_url:     cfg.endpoint.trim_end_matches('/').to_string(),
            project:      cfg.project.clone(),
            image_alias:  cfg.image_alias.clone(),
            image_server: cfg.image_server.clone(),
            profile:      cfg.profile.clone(),
        })
    }

    #[cfg(test)]
    fn test_client(base_url: &str) -> Self {
        Self {
            http:         Client::new(),
            base_url:     base_url.trim_end_matches('/').to_string(),
            project:      "default".into(),
            image_alias:  "24.04".into(),
            image_server: "https://cloud-images.ubuntu.com/releases".into(),
            profile:      "default".into(),
        }
    }

    pub fn project(&self) -> &str {
        &self.project
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn project_qs(&self) -> String {
        format!("project={}", self.project)
    }

    async fn wait_for_operation(&self, envelope: &Value, timeout_secs: u64) -> Result<Value> {
        let op_id = envelope["metadata"]["id"].as_str()
            .ok_or_else(|| anyhow::anyhow!("LXD response missing operation id: {envelope}"))?;
        let url = self.url(&format!("/1.0/operations/{op_id}/wait?timeout={timeout_secs}"));

        let resp = self.http.get(&url).send().await.context("waiting for LXD operation")?;
        let status = resp.status();
        let body: Value = resp.json().await.context("parsing LXD operation-wait response")?;
        if !status.is_success() {
            bail!("LXD operation wait failed ({status}): {body}");
        }

        let operation = body["metadata"].clone();
        if operation["status"].as_str() == Some("Failure") {
            let err = operation["err"].as_str().unwrap_or("unknown error");
            bail!("LXD operation failed: {err}");
        }
        Ok(operation)
    }

    async fn run_async(&self, resp: reqwest::Response, timeout_secs: u64) -> Result<Value> {
        let status = resp.status();
        let envelope: Value = resp.json().await.context("parsing LXD response")?;
        if status == StatusCode::ACCEPTED {
            return self.wait_for_operation(&envelope, timeout_secs).await;
        }
        if !status.is_success() {
            bail!("LXD request failed ({status}): {envelope}");
        }
        Ok(envelope["metadata"].clone())
    }

    pub async fn ensure_network(&self, name: &str) -> Result<()> {
        let get_url = self.url(&format!("/1.0/networks/{name}?{}", self.project_qs()));
        tracing::debug!(url = %get_url, "LXD GET (check network)");
        let resp = self.http.get(&get_url).send().await.context("checking LXD network")?;
        let status = resp.status();
        tracing::debug!(url = %get_url, %status, "LXD GET (check network) response");
        if status.is_success() {
            return Ok(());
        }
        if status != StatusCode::NOT_FOUND {
            let body: Value = resp.json().await.unwrap_or_default();
            bail!("LXD network lookup failed ({status}): {body}");
        }

        let create_url = self.url(&format!("/1.0/networks?{}", self.project_qs()));
        tracing::debug!(url = %create_url, network = name, "LXD POST (create network)");
        let resp = self.http.post(&create_url)
            .json(&json!({ "name": name, "type": "bridge" }))
            .send().await.context("creating LXD network")?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or_default();
        tracing::debug!(url = %create_url, %status, %body, "LXD POST (create network) response");
        if status.is_success() {
            return Ok(());
        }
        if status == StatusCode::CONFLICT || status == StatusCode::BAD_REQUEST {
            let msg = body["error"].as_str().unwrap_or("");
            if msg.to_lowercase().contains("already exists") {
                return Ok(());
            }
        }
        bail!("LXD network create failed ({status}): {body}");
    }

    pub async fn create_instance(&self, req: &CreateInstance) -> Result<()> {
        let url = self.url(&format!("/1.0/instances?{}", self.project_qs()));
        let body = json!({
            "name":        req.name,
            "description": req.description,
            "type":        "container",
            "profiles":    [self.profile],
            "source": {
                "type":     "image",
                "alias":    self.image_alias,
                "server":   self.image_server,
                "protocol": "simplestreams",
            },
            "config": {
                "limits.cpu":            req.flavor.cpu().to_string(),
                "limits.memory":         format!("{}MiB", req.flavor.memory_mib()),
                "user.harvest-managed":  "true",
            },
            "devices": {
                "eth0": { "type": "nic", "network": req.network, "name": "eth0" },
            },
        });

        tracing::debug!(url = %url, project = %self.project, profile = %self.profile, image_alias = %self.image_alias, "LXD POST (create instance)");
        let resp = self.http.post(&url).json(&body).send().await.context("creating LXD instance")?;
        let status = resp.status();
        tracing::debug!(url = %url, %status, "LXD POST (create instance) initial response");
        self.run_async(resp, DEFAULT_OPERATION_WAIT_SECS).await?;
        Ok(())
    }

    async fn set_instance_state(&self, name: &str, action: &str, timeout: u64) -> Result<()> {
        let url = self.url(&format!("/1.0/instances/{name}/state?{}", self.project_qs()));
        tracing::debug!(url = %url, action, "LXD PUT (set instance state)");
        let resp = self.http.put(&url)
            .json(&json!({ "action": action, "timeout": timeout, "force": action == "stop" }))
            .send().await.context("setting LXD instance state")?;
        let status = resp.status();
        tracing::debug!(url = %url, action, %status, "LXD PUT (set instance state) initial response");
        self.run_async(resp, timeout + 30).await?;
        Ok(())
    }

    pub async fn start_instance(&self, name: &str) -> Result<()> {
        self.set_instance_state(name, "start", 30).await
    }

    pub async fn stop_instance(&self, name: &str) -> Result<()> {
        self.set_instance_state(name, "stop", 30).await
    }

    pub async fn restart_instance(&self, name: &str) -> Result<()> {
        self.set_instance_state(name, "restart", 30).await
    }

    pub async fn wait_running(&self, name: &str, timeout_secs: u64) -> Result<()> {
        tracing::debug!(name, timeout_secs, "LXD wait_running: polling instance state");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            let url = self.url(&format!("/1.0/instances/{name}/state?{}", self.project_qs()));
            let resp = self.http.get(&url).send().await.context("checking LXD instance state")?;
            if resp.status().is_success() {
                let body: Value = resp.json().await.context("parsing LXD instance state")?;
                let instance_status = body["metadata"]["status"].as_str().unwrap_or("");
                tracing::debug!(name, instance_status, "LXD wait_running: poll result");
                if instance_status == "Running" {
                    return Ok(());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::error!(name, timeout_secs, "LXD wait_running: timed out");
                bail!("timed out waiting for instance {name} to reach Running state");
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    pub async fn exec(&self, name: &str, command: Vec<String>) -> Result<i64> {
        let url = self.url(&format!("/1.0/instances/{name}/exec?{}", self.project_qs()));
        tracing::debug!(url = %url, name, "LXD POST (exec)");
        let resp = self.http.post(&url)
            .json(&json!({
                "command":            command,
                "wait-for-websocket": false,
                "record-output":      true,
                "interactive":        false,
            }))
            .send().await.context("executing command in LXD instance")?;
        let status = resp.status();
        tracing::debug!(url = %url, %status, "LXD POST (exec) initial response");
        let operation = self.run_async(resp, DEFAULT_OPERATION_WAIT_SECS).await?;
        let exit_code = operation["metadata"]["return"].as_i64()
            .ok_or_else(|| anyhow::anyhow!("LXD exec response missing exit code: {operation}"))?;
        tracing::debug!(name, exit_code, "LXD exec finished");
        Ok(exit_code)
    }

    pub async fn exec_with_retry(
        &self,
        name: &str,
        command: Vec<String>,
        attempts: u32,
        delay: Duration,
        on_retry: impl Fn(u32, u32) + Send,
    ) -> Result<i64> {
        let mut last: Result<i64> = Err(anyhow::anyhow!("exec_with_retry called with 0 attempts"));
        for attempt in 1..=attempts {
            last = self.exec(name, command.clone()).await;
            let succeeded = matches!(last, Ok(0));
            if succeeded {
                return last;
            }
            if attempt < attempts {
                tracing::warn!(name, attempt, attempts, result = ?last, "exec attempt did not succeed, retrying");
                on_retry(attempt, attempts);
                tokio::time::sleep(delay).await;
            }
        }
        last
    }

    pub async fn delete_instance(&self, name: &str) -> Result<()> {
        tracing::debug!(name, "LXD delete_instance: stopping instance first (best-effort)");
        let _ = self.stop_instance(name).await;

        let url = self.url(&format!("/1.0/instances/{name}?{}", self.project_qs()));
        tracing::debug!(url = %url, name, "LXD DELETE (instance)");
        let resp = self.http.delete(&url).send().await.context("deleting LXD instance")?;
        let status = resp.status();
        tracing::debug!(url = %url, %status, "LXD DELETE (instance) response");
        if status == StatusCode::NOT_FOUND {
            return Ok(());
        }
        self.run_async(resp, 30).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    const TEST_CERT: &str = "-----BEGIN CERTIFICATE-----\nMIIDFzCCAf+gAwIBAgIUfa1whA4eLVBTEndTkfzURCCkn1gwDQYJKoZIhvcNAQEL\nBQAwGzEZMBcGA1UEAwwQaGFydmVzdC1seGQtdGVzdDAeFw0yNjA3MDMyMDA1MzVa\nFw0zNjA2MzAyMDA1MzVaMBsxGTAXBgNVBAMMEGhhcnZlc3QtbHhkLXRlc3QwggEi\nMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQC6adiI5Nwy9MDlzR92GgFc7b2L\n/ka6ccF3I6RyUxfUveDyY7WzkFOjWS6gWrM0mg+zTkP0EyEWdLaKZsZokum1gDIl\nNtd3A96d8Kz4MYxf7s+d7P+5NOyS0XaqwZmqRxkq+Ps/xrULcuBydLnkDse43+DU\n1AoueIiHZL2lDgGU0LhTUi8COln/daye2zjGv7dzaOzJCRq4YWc+D45ha8ii3GAr\natzLR8OjS9eMFrVivT5PLvGArp7qzVGvgQZ4AhTw9DELACt6gF85y4NOZ3+QhQb6\nVKza5MkNmBC6piEmOgBxOEzVwBwTw0PoQfESgGi6i2sWf5+jysCtLZAdHsSvAgMB\nAAGjUzBRMB0GA1UdDgQWBBQKFOj0BLCTRijKht/LnCciyhht1TAfBgNVHSMEGDAW\ngBQKFOj0BLCTRijKht/LnCciyhht1TAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3\nDQEBCwUAA4IBAQBTpVu67w2AzGjw2rWk32ZQp05ldBsc1PR3l/E0gxOhip4zpc3l\nOEg9KZtwtn9zkIwbRE8xDsKMY2acy1AroqcR0IUA/XXZQOXWjqKlQJRYePcIju6v\ngy636poDLDVWT09GzFEdh7adOll24i2ghRKxX+1gP6yoK7VKbh3u1K7SAPX4Cw+V\nUsTKgdy0ott1Calzr1rgLRDYxy2sAjtT98HQs+06J+JioHfkVowVBBsf8QV9Qq/d\nZCdcQ+Ej+sDvw5h5ynKUvm+LKHk6d6aKyh8cNL67Lz14znBoX/RElz/UnhSgvlc1\nklOERsYlhYR8s0qCGY8QIJfCCdOlvsuLMS5+\n-----END CERTIFICATE-----\n";
    const TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQC6adiI5Nwy9MDl\nzR92GgFc7b2L/ka6ccF3I6RyUxfUveDyY7WzkFOjWS6gWrM0mg+zTkP0EyEWdLaK\nZsZokum1gDIlNtd3A96d8Kz4MYxf7s+d7P+5NOyS0XaqwZmqRxkq+Ps/xrULcuBy\ndLnkDse43+DU1AoueIiHZL2lDgGU0LhTUi8COln/daye2zjGv7dzaOzJCRq4YWc+\nD45ha8ii3GAratzLR8OjS9eMFrVivT5PLvGArp7qzVGvgQZ4AhTw9DELACt6gF85\ny4NOZ3+QhQb6VKza5MkNmBC6piEmOgBxOEzVwBwTw0PoQfESgGi6i2sWf5+jysCt\nLZAdHsSvAgMBAAECggEAMl92zWs2m6hq1c5LoaDeXGu77C/+kdQ6iMS/Y8tTZcAX\noLhT+d1W1I29XUSVJ3I4KuZL05E1wDkyuIyUMd79O3gUVN0QdU884WYPf5P4EFZa\nkRzhb30/Ll9e1z6wlQRYZzXXwwChnKHix9sF/nwF+U26FhjkVXFpx1hwLMFvqPQV\nozsQGU2yxLpK1BU8p6peSYcbNy/klCizHdFrUQfI8YP4Vx6a1S7An1C7S857Xcfz\nH/QA8w7RZrW5zNznIyXM+gPC3nzY45FRBXKP8QVirYJi+DaJPCwfH9Kb7Vm090dr\nAMh9koJa1pY75bv7e6PDw1JRhRfbKBDnkn27gtY8qQKBgQDksHSZAvVJQMvPSXwr\nI+SUwDquE1hH72iRGpk3Fv1KuBqDQXvK+ITY6SL2T0VjRVFZEaX6iaPQJ8nyxBpc\neGYi7Y+UqlCN++O8Kze+yMTsQcbL5xzs2rZnZS3Sc4JMJMgn2f8+9GrPFTuSId0h\nxEGTBiG2HwPZ2J+JXwVglKp/aQKBgQDQrOx4aJwuOHyNjibRX1zjkv1f41yB+lrz\nBxI0phmNNr50Gk9fez0+CIokZbkGmHKj7K0ctR0siqKC7UyK/Zuj1ym0z6aRXhzm\nQ5lORHcA7ODYRSlYiEZUeamZI32gCnPB4NDlpod6Yqtr5b/iNjHy9/vrZnevck9S\n7JA41Yq4VwKBgB3+1wxKywl0qkbiCJtP9edc31V9zBKDYF/H8Vi8dzSZuUCGEkqp\nFiOtUJymAR/oM6dPHUojS4096ssg1aRTVnI2XqLNRAubgl9n+8PWaZ3jcsPD6JNY\njJw7NStpYynBmU9A1K3ZOTk4O7wLHQoUx9UU9M8CemrUcvh9siLc3RAhAoGAGGcg\ngDQ7j2wrpKIrB/EO+84Es2HzP3/3gtQg3OdPtaPhQdKR1aij0M1O2lLLAGpzfZf/\n5ouHjd3og0cc3GQr/0z6I5rk77sBxivBkdWP1Rveb2wnGaNWFirkGnR8DGssfk+8\nHh8LWNSRF10Ww21zCebWHwEsnefQPvJLK1pNjqECgYAHxsOV2AVLz6r3M4tkJjYq\nVIzo2ywbToKjsgX+SRkqxZJBzX43syl6tvhOHmuoYT/LKF/WFN0QvSs9BatpHqwQ\nTcWHxjiUYULgFHxiIXZCD2cMLaweYyUpDtXmXLOT2bv1Qd4glGSm/p+w71oG9Dga\nO1iD+apEEk/2MzDMH7ShKw==\n-----END PRIVATE KEY-----\n";

    fn test_lxd_config(endpoint: String) -> LxdConfig {
        LxdConfig {
            endpoint,
            client_cert: Some(TEST_CERT.to_string()),
            client_key:  Some(TEST_KEY.to_string()),
            trust_token: None,
            ca_cert:     None,
            insecure:    false,
            project:     "default".into(),
            image_alias: "24.04".into(),
            image_server: "https://cloud-images.ubuntu.com/releases".into(),
            profile:     "default".into(),
        }
    }

    fn sync_op(metadata: Value) -> Value {
        json!({ "type": "sync", "status": "Success", "status_code": 200, "metadata": metadata })
    }

    fn async_op(op_id: &str) -> Value {
        json!({
            "type": "async",
            "status": "Operation created",
            "status_code": 100,
            "metadata": { "id": op_id, "class": "task", "status": "Running", "status_code": 103 },
            "operation": format!("/1.0/operations/{op_id}"),
        })
    }

    fn op_wait_success(result: Value) -> Value {
        json!({
            "type": "sync",
            "status": "Success",
            "status_code": 200,
            "metadata": { "id": "op-1", "class": "task", "status": "Success", "status_code": 200, "err": "", "metadata": result },
        })
    }

    fn op_wait_failure(err: &str) -> Value {
        json!({
            "type": "sync",
            "status": "Success",
            "status_code": 200,
            "metadata": { "id": "op-1", "class": "task", "status": "Failure", "status_code": 400, "err": err, "metadata": {} },
        })
    }

    #[test]
    fn client_builds_from_valid_pem_identity() {
        let cfg = test_lxd_config("https://127.0.0.1:1".into());
        assert!(LxdClient::new(&cfg).is_ok());
    }

    #[test]
    fn client_rejects_malformed_pem() {
        let mut cfg = test_lxd_config("https://127.0.0.1:1".into());
        cfg.client_cert = Some("not a pem".into());
        cfg.client_key = Some("not a pem".into());
        assert!(LxdClient::new(&cfg).is_err());
    }

    #[tokio::test]
    async fn ensure_network_noop_when_already_exists() {
        let server = MockServer::start();
        let get_mock = server.mock(|when, then| {
            when.method("GET").path("/1.0/networks/hv-abc123").query_param("project", "default");
            then.status(200).json_body(sync_op(json!({ "name": "hv-abc123" })));
        });
        let create_mock = server.mock(|when, then| {
            when.method("POST").path("/1.0/networks");
            then.status(500);
        });

        let client = LxdClient::test_client(&server.base_url());
        client.ensure_network("hv-abc123").await.unwrap();

        get_mock.assert();
        create_mock.assert_calls(0);
    }

    #[tokio::test]
    async fn ensure_network_creates_when_missing() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("GET").path("/1.0/networks/hv-abc123");
            then.status(404).json_body(json!({ "error": "not found", "error_code": 404 }));
        });
        let create_mock = server.mock(|when, then| {
            when.method("POST").path("/1.0/networks").query_param("project", "default");
            then.status(200).json_body(sync_op(json!({})));
        });

        let client = LxdClient::test_client(&server.base_url());
        client.ensure_network("hv-abc123").await.unwrap();

        create_mock.assert();
    }

    #[tokio::test]
    async fn ensure_network_tolerates_racing_create() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("GET").path("/1.0/networks/hv-abc123");
            then.status(404).json_body(json!({ "error": "not found" }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/1.0/networks");
            then.status(400).json_body(json!({ "error": "The network already exists" }));
        });

        let client = LxdClient::test_client(&server.base_url());
        client.ensure_network("hv-abc123").await.unwrap();
    }

    #[tokio::test]
    async fn create_instance_waits_for_operation_success() {
        let server = MockServer::start();
        let create_mock = server.mock(|when, then| {
            when.method("POST").path("/1.0/instances").query_param("project", "default");
            then.status(202).json_body(async_op("op-create"));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-create/wait");
            then.status(200).json_body(op_wait_success(json!({})));
        });

        let client = LxdClient::test_client(&server.base_url());
        let req = CreateInstance {
            name: "agent-1".into(),
            network: "hv-abc123".into(),
            description: "test agent".into(),
            flavor: Flavor::Medium,
        };
        client.create_instance(&req).await.unwrap();
        create_mock.assert();
    }

    #[tokio::test]
    async fn create_instance_surfaces_operation_failure() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/1.0/instances");
            then.status(202).json_body(async_op("op-fail"));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-fail/wait");
            then.status(200).json_body(op_wait_failure("no space left on device"));
        });

        let client = LxdClient::test_client(&server.base_url());
        let req = CreateInstance {
            name: "agent-1".into(),
            network: "hv-abc123".into(),
            description: "".into(),
            flavor: Flavor::Tiny,
        };
        let err = client.create_instance(&req).await.unwrap_err();
        assert!(err.to_string().contains("no space left on device"));
    }

    #[tokio::test]
    async fn wait_running_polls_until_running() {
        let server = MockServer::start();
        let starting = server.mock(|when, then| {
            when.method("GET").path("/1.0/instances/agent-1/state");
            then.status(200).json_body(sync_op(json!({ "status": "Starting" })));
        });

        let client = LxdClient::test_client(&server.base_url());
        let result = tokio::time::timeout(
            Duration::from_millis(700),
            client.wait_running("agent-1", 5),
        ).await;
        assert!(result.is_err() || result.unwrap().is_err());
        assert!(starting.calls() >= 1);
    }

    #[tokio::test]
    async fn wait_running_succeeds_once_running() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("GET").path("/1.0/instances/agent-1/state");
            then.status(200).json_body(sync_op(json!({ "status": "Running" })));
        });

        let client = LxdClient::test_client(&server.base_url());
        client.wait_running("agent-1", 5).await.unwrap();
    }

    #[tokio::test]
    async fn exec_returns_exit_code() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/1.0/instances/agent-1/exec");
            then.status(202).json_body(async_op("op-exec"));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-exec/wait");
            then.status(200).json_body(op_wait_success(json!({ "return": 0 })));
        });

        let client = LxdClient::test_client(&server.base_url());
        let code = client.exec("agent-1", vec!["bash".into(), "-c".into(), "true".into()]).await.unwrap();
        assert_eq!(code, 0);
    }

    #[tokio::test]
    async fn exec_returns_nonzero_exit_code() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/1.0/instances/agent-1/exec");
            then.status(202).json_body(async_op("op-exec"));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-exec/wait");
            then.status(200).json_body(op_wait_success(json!({ "return": 1 })));
        });

        let client = LxdClient::test_client(&server.base_url());
        let code = client.exec("agent-1", vec!["false".into()]).await.unwrap();
        assert_eq!(code, 1);
    }

    #[tokio::test]
    async fn exec_with_retry_succeeds_on_first_try_without_retrying() {
        let server = MockServer::start();
        let exec_mock = server.mock(|when, then| {
            when.method("POST").path("/1.0/instances/agent-1/exec");
            then.status(202).json_body(async_op("op-exec"));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-exec/wait");
            then.status(200).json_body(op_wait_success(json!({ "return": 0 })));
        });

        let client = LxdClient::test_client(&server.base_url());
        let code = client
            .exec_with_retry("agent-1", vec!["true".into()], 3, Duration::from_millis(1), |_, _| {})
            .await.unwrap();

        assert_eq!(code, 0);
        exec_mock.assert_calls(1);
    }

    #[tokio::test]
    async fn exec_with_retry_exhausts_attempts_on_persistent_failure() {
        let server = MockServer::start();
        let exec_mock = server.mock(|when, then| {
            when.method("POST").path("/1.0/instances/agent-1/exec");
            then.status(202).json_body(async_op("op-exec"));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-exec/wait");
            then.status(200).json_body(op_wait_success(json!({ "return": 6 })));
        });

        let client = LxdClient::test_client(&server.base_url());
        let code = client
            .exec_with_retry("agent-1", vec!["curl".into()], 3, Duration::from_millis(1), |_, _| {})
            .await.unwrap();

        assert_eq!(code, 6, "should surface the last attempt's exit code once attempts are exhausted");
        exec_mock.assert_calls(3);
    }

    #[tokio::test]
    async fn exec_with_retry_retries_on_transport_error_too() {
        let server = MockServer::start();
        let exec_mock = server.mock(|when, then| {
            when.method("POST").path("/1.0/instances/agent-1/exec");
            then.status(500).json_body(json!({ "error": "boom" }));
        });

        let client = LxdClient::test_client(&server.base_url());
        let result = client
            .exec_with_retry("agent-1", vec!["true".into()], 2, Duration::from_millis(1), |_, _| {})
            .await;

        assert!(result.is_err());
        exec_mock.assert_calls(2);
    }

    #[tokio::test]
    async fn exec_with_retry_invokes_callback_with_attempt_and_total() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/1.0/instances/agent-1/exec");
            then.status(202).json_body(async_op("op-exec"));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-exec/wait");
            then.status(200).json_body(op_wait_success(json!({ "return": 6 })));
        });

        let client = LxdClient::test_client(&server.base_url());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = std::sync::Arc::clone(&seen);

        client
            .exec_with_retry("agent-1", vec!["curl".into()], 3, Duration::from_millis(1), move |attempt, attempts| {
                seen2.lock().unwrap().push((attempt, attempts));
            })
            .await.unwrap();

        assert_eq!(*seen.lock().unwrap(), vec![(1, 3), (2, 3)]);
    }

    #[tokio::test]
    async fn restart_instance_sends_restart_action() {
        let server = MockServer::start();
        let restart_mock = server.mock(|when, then| {
            when.method("PUT")
                .path("/1.0/instances/agent-1/state")
                .json_body_includes(r#"{"action":"restart"}"#);
            then.status(202).json_body(async_op("op-restart"));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-restart/wait");
            then.status(200).json_body(op_wait_success(json!({})));
        });

        let client = LxdClient::test_client(&server.base_url());
        client.restart_instance("agent-1").await.unwrap();
        restart_mock.assert();
    }

    #[tokio::test]
    async fn delete_instance_tolerates_already_gone() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("PUT").path("/1.0/instances/agent-1/state");
            then.status(404).json_body(json!({ "error": "not found" }));
        });
        let delete_mock = server.mock(|when, then| {
            when.method("DELETE").path("/1.0/instances/agent-1");
            then.status(404).json_body(json!({ "error": "not found" }));
        });

        let client = LxdClient::test_client(&server.base_url());
        client.delete_instance("agent-1").await.unwrap();
        delete_mock.assert();
    }

    #[tokio::test]
    async fn delete_instance_waits_for_delete_operation() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("PUT").path("/1.0/instances/agent-1/state");
            then.status(202).json_body(async_op("op-stop"));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-stop/wait");
            then.status(200).json_body(op_wait_success(json!({})));
        });
        server.mock(|when, then| {
            when.method("DELETE").path("/1.0/instances/agent-1");
            then.status(202).json_body(async_op("op-delete"));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-delete/wait");
            then.status(200).json_body(op_wait_success(json!({})));
        });

        let client = LxdClient::test_client(&server.base_url());
        client.delete_instance("agent-1").await.unwrap();
    }

    #[test]
    fn flavor_from_id_round_trips() {
        for f in Flavor::all() {
            assert_eq!(Flavor::from_id(f.id()), Some(*f));
        }
        assert_eq!(Flavor::from_id("bogus"), None);
    }

    #[test]
    fn flavor_json_has_expected_fields() {
        let json = Flavor::Medium.to_json();
        assert_eq!(json["id"], "medium");
        assert_eq!(json["cpu"], 2);
        assert_eq!(json["memory_mib"], 2048);
    }

    #[test]
    fn wants_manual_cert_true_when_both_set() {
        let cfg = test_lxd_config("https://lxd.example.com:8443".into());
        assert!(wants_manual_cert(&cfg));
    }

    #[test]
    fn wants_manual_cert_false_when_absent() {
        let mut cfg = test_lxd_config("https://lxd.example.com:8443".into());
        cfg.client_cert = None;
        cfg.client_key = None;
        assert!(!wants_manual_cert(&cfg));
    }

    #[test]
    fn wants_manual_cert_false_when_only_one_set() {
        let mut cfg = test_lxd_config("https://lxd.example.com:8443".into());
        cfg.client_key = None;
        assert!(!wants_manual_cert(&cfg));
    }

}

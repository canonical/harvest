use std::collections::HashMap;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::StatusCode;
use serde_json::{json, Value};

use super::LxdClient;

pub enum Source<'a> {
    Base,
    Image(&'a str),
    Copy(&'a str),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecOutput {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
}

impl ExecOutput {
    pub fn succeeded(&self) -> bool {
        self.exit_code == 0
    }
}

pub struct Command<'a> {
    pub argv: Vec<String>,
    pub cwd: Option<&'a str>,
    pub env: HashMap<String, String>,
    pub timeout: Duration,
}

impl<'a> Command<'a> {
    pub fn shell(script: &str) -> Self {
        Self {
            argv: vec!["bash".into(), "-lc".into(), script.into()],
            cwd: None,
            env: HashMap::new(),
            timeout: Duration::from_secs(120),
        }
    }

    pub fn in_dir(mut self, cwd: &'a str) -> Self {
        self.cwd = Some(cwd);
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl LxdClient {
    fn source_json(&self, source: &Source<'_>) -> Value {
        match source {
            Source::Base => json!({
                "type": "image",
                "alias": self.image_alias,
                "server": self.image_server,
                "protocol": "simplestreams",
            }),
            Source::Image(alias) => json!({ "type": "image", "alias": alias }),
            Source::Copy(instance) => json!({ "type": "copy", "source": instance, "instance_only": true }),
        }
    }

    pub async fn launch(&self, name: &str, source: Source<'_>, labels: &[(&str, &str)]) -> Result<()> {
        let mut config = serde_json::Map::new();
        config.insert("user.harvest-managed".into(), json!("true"));
        for (key, value) in labels {
            config.insert(format!("user.harvest-{key}"), json!(value));
        }
        let body = json!({
            "name": name,
            "type": "container",
            "profiles": [self.profile],
            "source": self.source_json(&source),
            "config": config,
        });
        let resp = self
            .http
            .post(self.url(&format!("/1.0/instances?{}", self.project_qs())))
            .json(&body)
            .send()
            .await
            .context("creating LXD container")?;
        self.run_async(resp, 600).await?;
        self.start_instance(name).await?;
        self.wait_running(name, 120).await
    }

    pub async fn publish(&self, instance: &str, alias: &str) -> Result<()> {
        self.delete_image_alias(alias).await?;
        let resp = self
            .http
            .post(self.url(&format!("/1.0/images?{}", self.project_qs())))
            .json(&json!({
                "source": { "type": "instance", "name": instance },
                "aliases": [{ "name": alias }],
            }))
            .send()
            .await
            .context("publishing LXD image")?;
        self.run_async(resp, 1800).await?;
        Ok(())
    }

    pub async fn delete_image_alias(&self, alias: &str) -> Result<()> {
        let resp = self
            .http
            .get(self.url(&format!("/1.0/images/aliases/{alias}?{}", self.project_qs())))
            .send()
            .await
            .context("looking up LXD image alias")?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        let body: Value = resp.json().await.context("parsing LXD image alias")?;
        let Some(fingerprint) = body["metadata"]["target"].as_str() else {
            return Ok(());
        };
        let resp = self
            .http
            .delete(self.url(&format!("/1.0/images/{fingerprint}?{}", self.project_qs())))
            .send()
            .await
            .context("deleting LXD image")?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        self.run_async(resp, 120).await?;
        Ok(())
    }

    pub async fn run(&self, name: &str, command: Command<'_>) -> Result<ExecOutput> {
        let mut body = json!({
            "command": command.argv,
            "environment": command.env,
            "wait-for-websocket": false,
            "record-output": true,
            "interactive": false,
        });
        if let Some(cwd) = command.cwd {
            body["cwd"] = json!(cwd);
        }
        let resp = self
            .http
            .post(self.url(&format!("/1.0/instances/{name}/exec?{}", self.project_qs())))
            .json(&body)
            .send()
            .await
            .context("running a command in the container")?;
        let operation = self.run_async(resp, command.timeout.as_secs().max(1)).await?;
        let metadata = &operation["metadata"];
        let exit_code = metadata["return"].as_i64().unwrap_or(-1);
        let stdout = self.read_log(&metadata["output"]["1"]).await?;
        let stderr = self.read_log(&metadata["output"]["2"]).await?;
        Ok(ExecOutput {
            exit_code,
            stdout,
            stderr,
        })
    }

    async fn read_log(&self, path: &Value) -> Result<String> {
        let Some(path) = path.as_str() else {
            return Ok(String::new());
        };
        let separator = if path.contains('?') { '&' } else { '?' };
        let url = self.url(&format!("{path}{separator}{}", self.project_qs()));
        let resp = self.http.get(&url).send().await.context("reading command output")?;
        let text = resp.text().await.context("reading command output")?;
        let _ = self.http.delete(&url).send().await;
        Ok(text)
    }

    pub async fn push_file(&self, name: &str, path: &str, contents: Vec<u8>, mode: u32) -> Result<()> {
        let resp = self
            .http
            .post(self.url(&format!("/1.0/instances/{name}/files")))
            .query(&[("path", path), ("project", self.project())])
            .header("X-LXD-type", "file")
            .header("X-LXD-uid", "0")
            .header("X-LXD-gid", "0")
            .header("X-LXD-mode", format!("{mode:o}"))
            .header("X-LXD-write", "overwrite")
            .body(contents)
            .send()
            .await
            .context("writing a file into the container")?;
        if !resp.status().is_success() {
            bail!("could not write {path}: {}", resp.text().await.unwrap_or_default());
        }
        Ok(())
    }

    pub async fn pull_file(&self, name: &str, path: &str) -> Result<Vec<u8>> {
        let resp = self
            .http
            .get(self.url(&format!("/1.0/instances/{name}/files")))
            .query(&[("path", path), ("project", self.project())])
            .send()
            .await
            .context("reading a file from the container")?;
        if !resp.status().is_success() {
            bail!("could not read {path}: {}", resp.status());
        }
        Ok(resp.bytes().await?.to_vec())
    }

    pub async fn ipv4(&self, name: &str) -> Result<Option<String>> {
        let resp = self
            .http
            .get(self.url(&format!("/1.0/instances/{name}/state?{}", self.project_qs())))
            .send()
            .await
            .context("reading the container state")?;
        let body: Value = resp.json().await.context("parsing the container state")?;
        let addresses = body["metadata"]["network"]["eth0"]["addresses"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(addresses
            .iter()
            .find(|a| a["family"] == "inet" && a["scope"] == "global")
            .and_then(|a| a["address"].as_str())
            .map(String::from))
    }

    pub async fn wait_for_ipv4(&self, name: &str, timeout: Duration) -> Result<String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(address) = self.ipv4(name).await? {
                return Ok(address);
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("container {name} got no network address");
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

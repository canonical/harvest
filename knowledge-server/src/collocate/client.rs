use anyhow::{anyhow, Context, Result};
use collocate_remote::client::Endpoint;
use collocate_remote::control::{Collocate, ExecOptions, LogWindow, Logs};
use collocate_remote::{ContainerId, ContainerInfo, ImageMeta};
use collocate_trust::Identity;
use std::io::Cursor;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::CollocateConfig;

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Default)]
pub struct CreateContainerOpts {
    pub image: Option<String>,
    pub command: Vec<String>,
    pub env: Vec<(String, String)>,
    pub workdir: Option<String>,
    pub user: Option<String>,
    pub publish: Vec<(u16, u16)>,
    pub bind_mounts: Vec<(String, String, bool)>,
    pub persistent: bool,
    pub idle_timeout_secs: Option<u64>,
}

impl CreateContainerOpts {
    fn apply_to_builder<'a>(self, b: &'a mut collocate_remote::control::RunBuilder<'a>, project: &str) -> &'a mut collocate_remote::control::RunBuilder<'a> {
        let image = self.image.as_deref().unwrap_or("");
        b.image(image).project(project).label("harvest.managed", "true");
        if !self.command.is_empty() {
            let argv: Vec<&str> = self.command.iter().map(|s| s.as_str()).collect();
            b.command(&argv);
        }
        for (k, v) in &self.env {
            b.env(k.clone(), v.clone());
        }
        if let Some(wd) = &self.workdir {
            b.workdir(wd.clone());
        }
        if let Some(u) = &self.user {
            b.user(u.clone());
        }
        for (host, container) in &self.publish {
            b.publish_tcp(*host, *container);
        }
        for (src, dst, ro) in &self.bind_mounts {
            b.bind(src.clone(), dst.clone(), *ro);
        }
        if self.persistent {
            b.persistent();
        }
        if let Some(secs) = self.idle_timeout_secs {
            b.idle_timeout(secs);
        }
        b
    }
}

#[derive(Clone)]
pub struct CollocateHandle {
    inner: Arc<Mutex<Collocate>>,
    config: CollocateConfig,
}

impl CollocateHandle {
    pub fn connect(config: CollocateConfig) -> Result<Self> {
        let (identity, server_fingerprint) = build_identity(&config)?;
        let endpoint = Endpoint {
            addresses: vec![config.endpoint.clone()],
            fingerprint: server_fingerprint,
        };
        let client = Collocate::from_endpoint(endpoint, identity);
        Ok(Self {
            inner: Arc::new(Mutex::new(client)),
            config,
        })
    }

    pub fn config(&self) -> &CollocateConfig {
        &self.config
    }

    pub async fn info(&self) -> Result<serde_json::Value> {
        let mut c = self.inner.lock().await;
        c.info().map_err(|e| anyhow!("collocate info: {e}"))
    }

    pub async fn pull(&self, reference: &str) -> Result<ImageMeta> {
        let mut c = self.inner.lock().await;
        c.pull(reference).map_err(|e| anyhow!("collocate pull {reference}: {e}"))
    }

    pub async fn images(&self) -> Result<Vec<ImageMeta>> {
        let mut c = self.inner.lock().await;
        c.images().map_err(|e| anyhow!("collocate images: {e}"))
    }

    pub async fn create_container(&self, name: &str, opts: CreateContainerOpts) -> Result<ContainerId> {
        let mut c = self.inner.lock().await;
        let mut builder = c.specify(name);
        let builder = opts.apply_to_builder(&mut builder, &self.config.project);
        builder
            .run()
            .map_err(|e| anyhow!("collocate run {name}: {e}"))
    }

    pub async fn exec(&self, target: &str, argv: &[&str], opts: &ExecOptions, stdin: Vec<u8>) -> Result<ExecResult> {
        let mut c = self.inner.lock().await;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = c
            .exec(target, argv, opts, Cursor::new(stdin), &mut stdout, &mut stderr)
            .map_err(|e| anyhow!("collocate exec in {target}: {e}"))?;
        Ok(ExecResult {
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            exit_code: exit,
        })
    }

    pub async fn exec_simple(&self, target: &str, argv: &[&str]) -> Result<ExecResult> {
        let opts = ExecOptions::new();
        self.exec(target, argv, &opts, Vec::new()).await
    }

    pub async fn exec_with_timeout(&self, target: &str, argv: &[&str], timeout_secs: u64) -> Result<ExecResult> {
        let opts = ExecOptions {
            timeout_secs: Some(timeout_secs),
            ..ExecOptions::new()
        };
        self.exec(target, argv, &opts, Vec::new()).await
    }

    pub async fn transfer_to(&self, target: &str, remote_path: &str, content: Vec<u8>) -> Result<()> {
        let opts = ExecOptions::new();
        let shell_cmd = format!("cat > '{remote_path}'");
        let argv: [&str; 3] = ["sh", "-c", &shell_cmd];
        let result = self.exec(target, &argv, &opts, content).await?;
        if result.exit_code != 0 {
            anyhow::bail!("transfer to {remote_path} failed: {}", result.stderr);
        }
        Ok(())
    }

    pub async fn transfer_from(&self, target: &str, remote_path: &str) -> Result<Vec<u8>> {
        let opts = ExecOptions::new();
        let result = self.exec(target, &["cat", remote_path], &opts, Vec::new()).await?;
        if result.exit_code != 0 {
            anyhow::bail!("transfer from {remote_path} failed: {}", result.stderr);
        }
        Ok(result.stdout.into_bytes())
    }

    pub async fn stop(&self, target: &str, timeout_secs: Option<u64>) -> Result<()> {
        let mut c = self.inner.lock().await;
        c.stop(target, timeout_secs)
            .map_err(|e| anyhow!("collocate stop {target}: {e}"))
    }

    pub async fn remove(&self, target: &str, force: bool) -> Result<()> {
        let mut c = self.inner.lock().await;
        c.remove(target, force, false)
            .map_err(|e| anyhow!("collocate remove {target}: {e}"))
    }

    pub async fn rm_force(&self, target: &str) -> Result<()> {
        let mut c = self.inner.lock().await;
        c.rm_force(target)
            .map_err(|e| anyhow!("collocate rm {target}: {e}"))
    }

    pub async fn list(&self) -> Result<Vec<ContainerInfo>> {
        let mut c = self.inner.lock().await;
        c.ps(true, Some(self.config.project.as_str()))
            .map_err(|e| anyhow!("collocate ps: {e}"))
    }

    pub async fn logs(&self, target: &str) -> Result<LogWindow> {
        let mut c = self.inner.lock().await;
        c.logs(target, &Logs::default())
            .map_err(|e| anyhow!("collocate logs {target}: {e}"))
    }

    pub async fn wait(&self, target: &str) -> Result<i32> {
        let mut c = self.inner.lock().await;
        c.wait(target)
            .map_err(|e| anyhow!("collocate wait {target}: {e}"))
    }
}

fn build_identity(config: &CollocateConfig) -> Result<(Identity, String)> {
    if let (Some(cert_path), Some(key_path)) = (&config.client_cert, &config.client_key) {
        let cert = std::fs::read_to_string(cert_path)
            .with_context(|| format!("reading client cert at {cert_path}"))?;
        let key = std::fs::read_to_string(key_path)
            .with_context(|| format!("reading client key at {key_path}"))?;
        let fingerprint = collocate_trust::fingerprint_pem(&cert)
            .map_err(|e| anyhow!("fingerprint_pem: {e}"))?;
        let identity = Identity {
            certificate: cert,
            key,
            fingerprint,
        };
        let server_fp = config.server_fingerprint.as_ref()
            .ok_or_else(|| anyhow!("collocate config with client_cert+client_key requires server_fingerprint"))?;
        return Ok((identity, server_fp.clone()));
    }
    if let Some(token) = &config.trust_token {
        let identity = generate_identity()?;
        let parsed = collocate_trust::Token::decode(token)
            .context("decoding collocate trust token")?;
        let server_fingerprint = parsed.fingerprint.clone();
        let _enrolled = collocate_remote::client::enroll(&parsed, &identity, Some("harvest"))
            .map_err(|e| anyhow!("collocate enroll: {e}"))?;
        return Ok((identity, server_fingerprint));
    }
    Err(anyhow!(
        "collocate config requires either client_cert+client_key or trust_token"
    ))
}

fn generate_identity() -> Result<Identity> {
    use rcgen::KeyPair;
    use rcgen::CertificateParams;
    use rcgen::DistinguishedName;
    use rcgen::DnType;

    let params = CertificateParams::new(vec!["harvest".to_string()])?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "harvest");
    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    let fingerprint = collocate_trust::fingerprint_pem(&cert_pem)
        .map_err(|e| anyhow!("fingerprint_pem: {e}"))?;
    Ok(Identity {
        certificate: cert_pem,
        key: key_pem,
        fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> CollocateConfig {
        CollocateConfig {
            endpoint: "10.0.0.1:8443".into(),
            trust_token: None,
            client_cert: None,
            client_key: None,
            project: "harvest".into(),
            default_image: "ubuntu:24.04".into(),
            default_idle_timeout_secs: 600,
            max_containers: 20,
            ca_cert: None,
            insecure: false,
            server_fingerprint: None,
        }
    }

    #[test]
    fn build_identity_fails_without_cert_or_token() {
        let config = make_config();
        assert!(build_identity(&config).is_err());
    }

    #[test]
    fn build_identity_succeeds_with_cert_and_key() {
        let keypair = rcgen::KeyPair::generate().unwrap();
        let params = rcgen::CertificateParams::new(vec!["test".to_string()]).unwrap();
        let cert = params.self_signed(&keypair).unwrap();
        let cert_pem = cert.pem();
        let key_pem = keypair.serialize_pem();

        let dir = std::env::temp_dir();
        let cert_path = dir.join("harvest_test_cert.pem");
        let key_path = dir.join("harvest_test_key.pem");
        std::fs::write(&cert_path, &cert_pem).unwrap();
        std::fs::write(&key_path, &key_pem).unwrap();

        let mut config = make_config();
        config.client_cert = Some(cert_path.to_string_lossy().into_owned());
        config.client_key = Some(key_path.to_string_lossy().into_owned());
        config.server_fingerprint = Some("abcdef0123456789".to_string());
        let (identity, server_fp) = build_identity(&config).unwrap();
        assert!(!identity.certificate.is_empty());
        assert!(!identity.key.is_empty());
        assert!(!identity.fingerprint.is_empty());
        assert_eq!(server_fp, "abcdef0123456789");
    }

    #[test]
    fn exec_result_fields_are_preserved() {
        let r = ExecResult {
            stdout: "hello".into(),
            stderr: "warn".into(),
            exit_code: 0,
        };
        assert_eq!(r.stdout, "hello");
        assert_eq!(r.stderr, "warn");
        assert_eq!(r.exit_code, 0);
    }

    #[test]
    fn create_container_opts_apply_does_not_panic() {
        let config = make_config();
        let mut c = Collocate::from_endpoint(
            Endpoint {
                addresses: vec![config.endpoint.clone()],
                fingerprint: "deadbeef".into(),
            },
            Identity {
                certificate: "".into(),
                key: "".into(),
                fingerprint: "deadbeef".into(),
            },
        );
        let opts = CreateContainerOpts {
            image: Some("ubuntu:24.04".into()),
            command: vec!["sleep".into(), "10".into()],
            env: vec![("FOO".into(), "bar".into())],
            workdir: Some("/tmp".into()),
            user: Some("root".into()),
            publish: vec![(8080, 80)],
            bind_mounts: vec![("/host".into(), "/container".into(), false)],
            persistent: true,
            idle_timeout_secs: Some(300),
            ..Default::default()
        };
        let mut builder = c.specify("test-ctr");
        opts.apply_to_builder(&mut builder, "harvest");
    }
}

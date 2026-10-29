use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use serde::Deserialize;

const CA_NAME: &str = "Harvest cluster CA";

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TlsPaths {
    pub ca_cert: PathBuf,
    pub cert: PathBuf,
    pub key: PathBuf,
}

pub struct ClusterTls {
    roots: Arc<RootCertStore>,
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    ca_pem: Vec<u8>,
    identity_pem: Vec<u8>,
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("reading {}", path.display()))
}

impl ClusterTls {
    pub fn load(paths: &TlsPaths) -> Result<Arc<Self>> {
        let ca_pem = read(&paths.ca_cert)?;
        let cert_pem = read(&paths.cert)?;
        let key_pem = read(&paths.key)?;
        let mut roots = RootCertStore::empty();
        for ca in CertificateDer::pem_slice_iter(&ca_pem) {
            roots.add(ca.context("parsing the cluster CA")?)?;
        }
        let chain = CertificateDer::pem_slice_iter(&cert_pem)
            .collect::<Result<Vec<_>, _>>()
            .context("parsing the node certificate")?;
        let key = PrivateKeyDer::from_pem_slice(&key_pem).context("parsing the node key")?;
        let mut identity_pem = cert_pem;
        identity_pem.extend_from_slice(&key_pem);
        Ok(Arc::new(Self {
            roots: Arc::new(roots),
            chain,
            key,
            ca_pem,
            identity_pem,
        }))
    }

    pub fn server_config(&self) -> Result<Arc<ServerConfig>> {
        let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
            Arc::clone(&self.roots),
            provider(),
        )
        .build()?;
        let mut config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()?
            .with_client_cert_verifier(verifier)
            .with_single_cert(self.chain.clone(), self.key.clone_key())?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Arc::new(config))
    }

    pub fn client_config(&self) -> Result<Arc<ClientConfig>> {
        Ok(Arc::new(
            ClientConfig::builder_with_provider(provider())
                .with_safe_default_protocol_versions()?
                .with_root_certificates(Arc::clone(&self.roots))
                .with_client_auth_cert(self.chain.clone(), self.key.clone_key())?,
        ))
    }

    pub fn http_client(&self, builder: reqwest::ClientBuilder) -> Result<reqwest::ClientBuilder> {
        Ok(builder
            .add_root_certificate(reqwest::Certificate::from_pem(&self.ca_pem)?)
            .identity(reqwest::Identity::from_pem(&self.identity_pem)?)
            .tls_built_in_root_certs(false))
    }

    pub fn websocket_connector(&self) -> Result<tokio_tungstenite::Connector> {
        Ok(tokio_tungstenite::Connector::Rustls(self.client_config()?))
    }
}

pub async fn serve<F>(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    config: Arc<ServerConfig>,
    stop: F,
) where
    F: std::future::Future<Output = ()>,
{
    use tower::ServiceExt as _;
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = &mut stop => return,
            accepted = listener.accept() => {
                let Ok((tcp, peer)) = accepted else {
                    continue;
                };
                let acceptor = acceptor.clone();
                let app = app.clone();
                tokio::spawn(async move {
                    let stream = match acceptor.accept(tcp).await {
                        Ok(stream) => stream,
                        Err(e) => {
                            tracing::debug!(error = %e, %peer, "rejected an internal connection");
                            return;
                        }
                    };
                    let service = hyper::service::service_fn(move |mut request: hyper::Request<hyper::body::Incoming>| {
                        request.extensions_mut().insert(axum::extract::ConnectInfo(peer));
                        app.clone().oneshot(request.map(axum::body::Body::new))
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                        .with_upgrades()
                        .await;
                });
            }
        }
    }
}

pub struct Issued {
    pub cert: PathBuf,
    pub key: PathBuf,
    pub ca_cert: PathBuf,
}

fn ca_params() -> Result<rcgen::CertificateParams> {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new())?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, CA_NAME);
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
    ];
    Ok(params)
}

fn write_private(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn certificate_authority(dir: &Path) -> Result<rcgen::KeyPair> {
    let cert = dir.join("ca.pem");
    let key = dir.join("ca-key.pem");
    if key.exists() {
        return Ok(rcgen::KeyPair::from_pem(&std::fs::read_to_string(&key)?)?);
    }
    let pair = rcgen::KeyPair::generate()?;
    let issued = ca_params()?.self_signed(&pair)?;
    std::fs::write(&cert, issued.pem())?;
    write_private(&key, &pair.serialize_pem())?;
    Ok(pair)
}

pub fn issue(dir: &Path, name: &str, hosts: &[String]) -> Result<Issued> {
    std::fs::create_dir_all(dir)?;
    let ca_key = certificate_authority(dir)?;
    let issuer = rcgen::Issuer::new(ca_params()?, ca_key);
    let mut params = rcgen::CertificateParams::new(hosts.to_vec())?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, name);
    params.extended_key_usages = vec![
        rcgen::ExtendedKeyUsagePurpose::ServerAuth,
        rcgen::ExtendedKeyUsagePurpose::ClientAuth,
    ];
    let pair = rcgen::KeyPair::generate()?;
    let leaf = params.signed_by(&pair, &issuer)?;
    let cert = dir.join(format!("{name}.pem"));
    let key = dir.join(format!("{name}-key.pem"));
    std::fs::write(&cert, leaf.pem())?;
    write_private(&key, &pair.serialize_pem())?;
    Ok(Issued {
        cert,
        key,
        ca_cert: dir.join("ca.pem"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_certificates_load_into_client_and_server_configs() {
        let dir = tempfile::tempdir().unwrap();
        let issued = issue(dir.path(), "node-a", &["127.0.0.1".into(), "node-a".into()]).unwrap();
        let tls = ClusterTls::load(&TlsPaths {
            ca_cert: issued.ca_cert,
            cert: issued.cert,
            key: issued.key,
        })
        .unwrap();
        assert!(tls.server_config().is_ok());
        assert!(tls.client_config().is_ok());
        assert!(tls
            .http_client(reqwest::Client::builder())
            .unwrap()
            .build()
            .is_ok());
    }

    #[test]
    fn every_node_is_issued_by_the_same_authority() {
        let dir = tempfile::tempdir().unwrap();
        issue(dir.path(), "node-a", &["a".into()]).unwrap();
        let ca_before = std::fs::read(dir.path().join("ca.pem")).unwrap();
        issue(dir.path(), "node-b", &["b".into()]).unwrap();
        assert_eq!(std::fs::read(dir.path().join("ca.pem")).unwrap(), ca_before);
    }
}

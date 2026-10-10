use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::CertificateDer;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_postgres::config::SslMode;
use tokio_postgres::{AsyncMessage, Client, Config, Connection, NoTls};
use tokio_postgres_rustls::MakeRustlsConnect;

pub const DEFAULT_POOL_SIZE: usize = 16;

#[derive(Clone, Debug)]
pub struct DbOptions {
    pub url: String,
    pub pool_size: usize,
    pub ca_file: Option<PathBuf>,
}

impl DbOptions {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into(), pool_size: DEFAULT_POOL_SIZE, ca_file: None }
    }

    pub fn with_pool_size(mut self, pool_size: usize) -> Self {
        self.pool_size = pool_size.max(1);
        self
    }

    pub fn with_ca_file(mut self, ca_file: Option<PathBuf>) -> Self {
        self.ca_file = ca_file;
        self
    }

    pub fn parsed(&self) -> Result<Config> {
        Config::from_str(&self.url).context("invalid database url")
    }

    pub fn requires_tls(&self) -> Result<bool> {
        Ok(self.ca_file.is_some() || self.parsed()?.get_ssl_mode() == SslMode::Require)
    }
}

#[derive(Clone)]
pub(crate) enum Tls {
    Plain,
    Rustls(MakeRustlsConnect),
}

impl Tls {
    pub(crate) fn from_options(options: &DbOptions) -> Result<Self> {
        if !options.requires_tls()? {
            return Ok(Self::Plain);
        }
        let mut roots = rustls::RootCertStore::empty();
        match &options.ca_file {
            Some(path) => {
                let certs = CertificateDer::pem_file_iter(path)
                    .with_context(|| format!("reading database CA file {}", path.display()))?
                    .collect::<Result<Vec<_>, _>>()
                    .with_context(|| format!("parsing database CA file {}", path.display()))?;
                if certs.is_empty() {
                    anyhow::bail!("database CA file {} contains no certificate", path.display());
                }
                for cert in certs {
                    roots.add(cert).with_context(|| format!("adding certificate from {}", path.display()))?;
                }
            }
            None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
        }
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .context("configuring TLS protocol versions")?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(Self::Rustls(MakeRustlsConnect::new(config)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub channel: String,
    pub payload: String,
}

pub struct Listener {
    client: Client,
    receiver: mpsc::UnboundedReceiver<Notification>,
    backend_pid: i32,
}

impl Listener {
    pub async fn recv(&mut self) -> Option<Notification> {
        self.receiver.recv().await
    }

    pub fn backend_pid(&self) -> i32 {
        self.backend_pid
    }

    pub fn client(&self) -> &Client {
        &self.client
    }
}

fn drive<S, T>(mut connection: Connection<S, T>, sender: Option<mpsc::UnboundedSender<Notification>>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut messages = futures_util::stream::poll_fn(move |cx| connection.poll_message(cx));
        while let Some(message) = messages.next().await {
            match message {
                Ok(AsyncMessage::Notification(n)) => {
                    if let Some(sender) = &sender {
                        let _ = sender.send(Notification {
                            channel: n.channel().to_string(),
                            payload: n.payload().to_string(),
                        });
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "dedicated database connection closed");
                    break;
                }
            }
        }
    });
}

pub(crate) async fn connect_dedicated(
    config: &Config,
    tls: &Tls,
    sender: Option<mpsc::UnboundedSender<Notification>>,
) -> Result<Client> {
    match tls {
        Tls::Plain => {
            let (client, connection) = config.connect(NoTls).await.context("could not open a database connection")?;
            drive(connection, sender);
            Ok(client)
        }
        Tls::Rustls(connector) => {
            let (client, connection) = config.connect(connector.clone()).await.context("could not open a database connection")?;
            drive(connection, sender);
            Ok(client)
        }
    }
}

pub(crate) async fn listen(config: &Config, tls: &Tls, channels: &[&str]) -> Result<Listener> {
    let (sender, receiver) = mpsc::unbounded_channel();
    let client = connect_dedicated(config, tls, Some(sender)).await?;
    for channel in channels {
        client
            .batch_execute(&format!("LISTEN \"{}\"", channel.replace('"', "\"\"")))
            .await
            .with_context(|| format!("LISTEN {channel}"))?;
    }
    let backend_pid: i32 = client.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);
    Ok(Listener { client, receiver, backend_pid })
}

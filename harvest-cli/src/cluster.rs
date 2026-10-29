use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use clap::{Parser, Subcommand};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    apply_snap_settings, generate_secret, non_empty_or, require_snapctl, run_configure_hook,
    snapctl_get, to_local_probe_host, validate_identifier,
};

const SECRET_HEADER: &str = "x-harvest-cluster-secret";
const DEFAULT_INTERNAL_LISTEN: &str = "0.0.0.0:8081";

#[derive(Parser)]
pub struct ClusterArgs {
    #[command(subcommand)]
    command: ClusterCommand,
}

#[derive(Subcommand)]
enum ClusterCommand {
    Init(InitArgs),
    Join(JoinArgs),
    Preseed(PreseedArgs),
    Status(JsonArgs),
    List(JsonArgs),
    Remove(TargetArgs),
    Evacuate(TargetArgs),
    Restore(TargetArgs),
    RotateSecrets(RotateArgs),
    Token(TokenArgs),
    Recover,
}

#[derive(Parser)]
struct NodeArgs {
    #[arg(long)]
    node_name: Option<String>,
    #[arg(long, default_value = DEFAULT_INTERNAL_LISTEN)]
    internal_listen: String,
    #[arg(long)]
    internal_url: Option<String>,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Parser)]
struct InitArgs {
    #[command(flatten)]
    node: NodeArgs,
    #[arg(long)]
    shared_secret: Option<String>,
}

#[derive(Parser)]
struct JoinArgs {
    #[command(flatten)]
    node: NodeArgs,
    #[arg(long)]
    token: String,
}

#[derive(Parser)]
struct PreseedArgs {
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Parser, Default)]
struct JsonArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct TargetArgs {
    target: String,
}

#[derive(Parser)]
struct RotateArgs {
    #[command(subcommand)]
    step: RotateStep,
}

#[derive(Subcommand)]
enum RotateStep {
    Stage {
        #[arg(long)]
        secret: Option<String>,
    },
    Commit {
        #[arg(long)]
        secret: String,
    },
    Finish,
}

#[derive(Parser)]
struct TokenArgs {
    #[command(subcommand)]
    command: TokenCommand,
}

#[derive(Subcommand)]
enum TokenCommand {
    Create {
        #[arg(long, default_value_t = 1800)]
        ttl_secs: u64,
        #[arg(long)]
        json: bool,
    },
    List(JsonArgs),
    Revoke,
}

#[derive(Deserialize)]
struct Preseed {
    cluster: PreseedCluster,
    systems: Vec<PreseedSystem>,
}

#[derive(Deserialize)]
struct PreseedCluster {
    shared_secret: String,
    internal_listen: Option<String>,
    bootstrap: Option<String>,
}

#[derive(Deserialize, Clone)]
struct PreseedSystem {
    name: String,
    address: String,
}

type Settings = Vec<(String, String)>;

pub async fn run(args: ClusterArgs) -> Result<()> {
    require_snapctl()?;
    match args.command {
        ClusterCommand::Init(args) => init(args),
        ClusterCommand::Join(args) => join(args),
        ClusterCommand::Preseed(args) => preseed(args),
        ClusterCommand::Status(args) => status(args).await,
        ClusterCommand::List(args) => list(args).await,
        ClusterCommand::Remove(args) => remove(args).await,
        ClusterCommand::Evacuate(args) => set_evacuated(args, true).await,
        ClusterCommand::Restore(args) => set_evacuated(args, false).await,
        ClusterCommand::RotateSecrets(args) => rotate_secrets(args),
        ClusterCommand::Token(args) => token(args).await,
        ClusterCommand::Recover => recover(),
    }
}

fn is_managed() -> Result<bool> {
    Ok(snapctl_get("config.managed")?
        .trim()
        .eq_ignore_ascii_case("true"))
}

fn ensure_mutable() -> Result<()> {
    if is_managed()? {
        bail!(
            "cluster is charm-managed. Use juju add-unit/remove-unit/actions for cluster changes"
        );
    }
    Ok(())
}

fn node_settings(node: &NodeArgs) -> Result<Settings> {
    let internal_url = match &node.internal_url {
        Some(url) => url.clone(),
        None => snapctl_get("cluster.internal-url")?,
    };
    if internal_url.trim().is_empty() {
        bail!("cluster internal URL is required (pass --internal-url)");
    }
    validate_cluster_url(&internal_url)?;
    let mut settings = vec![
        ("cluster.backend".to_string(), "quorium".to_string()),
        (
            "cluster.internal-listen".to_string(),
            node.internal_listen.clone(),
        ),
        ("cluster.internal-url".to_string(), internal_url),
    ];
    if let Some(name) = &node.node_name {
        validate_identifier("cluster node name", &name.replace('-', "_"))?;
        settings.push(("cluster.node-name".to_string(), name.clone()));
    }
    Ok(settings)
}

fn apply(mut settings: Settings, dry_run: bool) -> Result<()> {
    settings.sort_by(|a, b| a.0.cmp(&b.0));
    if dry_run {
        for (key, value) in settings {
            println!("{key}={value}");
        }
        return Ok(());
    }
    apply_snap_settings(&settings)?;
    run_configure_hook()
}

fn init(args: InitArgs) -> Result<()> {
    ensure_mutable()?;
    let mut settings = node_settings(&args.node)?;
    let secret = args.shared_secret.unwrap_or_else(|| generate_secret(48));
    settings.extend([
        ("cluster.bootstrap".to_string(), "init".to_string()),
        ("cluster.shared-secret".to_string(), secret),
        ("cluster.join-token".to_string(), String::new()),
        ("cluster.join-addresses".to_string(), String::new()),
    ]);
    apply(settings, args.node.dry_run)
}

fn join(args: JoinArgs) -> Result<()> {
    ensure_mutable()?;
    let secret = token_shared_secret(&args.token)?;
    let mut settings = node_settings(&args.node)?;
    settings.extend([
        ("cluster.bootstrap".to_string(), "join".to_string()),
        ("cluster.join-token".to_string(), args.token),
        ("cluster.shared-secret".to_string(), secret),
        ("cluster.join-addresses".to_string(), String::new()),
    ]);
    apply(settings, args.node.dry_run)
}

fn token_shared_secret(token: &str) -> Result<String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(token.trim())
        .context("the join token is not valid")?;
    let decoded: Value = serde_json::from_slice(&bytes).context("the join token is not valid")?;
    decoded["k"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("the join token carries no cluster secret"))
}

fn preseed(args: PreseedArgs) -> Result<()> {
    ensure_mutable()?;
    let input = match args.file {
        Some(path) => std::fs::read_to_string(path)?,
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };
    let preseed: Preseed = serde_yaml::from_str(&input).context("parsing the preseed")?;
    let local = snapctl_get("cluster.node-name")?;
    let local = non_empty_or(local, &std::env::var("HOSTNAME").unwrap_or_default());
    apply(preseed_settings(&preseed, &local)?, args.dry_run)
}

fn preseed_settings(preseed: &Preseed, local: &str) -> Result<Settings> {
    let system = preseed
        .systems
        .iter()
        .find(|s| s.name == local)
        .ok_or_else(|| anyhow!("the preseed does not list this node ({local})"))?;
    validate_cluster_url(&system.address)?;
    let seed = preseed
        .cluster
        .bootstrap
        .clone()
        .or_else(|| preseed.systems.first().map(|s| s.name.clone()))
        .ok_or_else(|| anyhow!("the preseed lists no systems"))?;
    let peers: Vec<String> = preseed
        .systems
        .iter()
        .filter(|s| s.name != local)
        .map(|s| s.address.clone())
        .collect();
    let bootstrap = if seed == local { "init" } else { "join" };
    Ok(vec![
        ("cluster.backend".to_string(), "quorium".to_string()),
        ("cluster.node-name".to_string(), local.to_string()),
        ("cluster.internal-url".to_string(), system.address.clone()),
        (
            "cluster.internal-listen".to_string(),
            preseed
                .cluster
                .internal_listen
                .clone()
                .unwrap_or_else(|| DEFAULT_INTERNAL_LISTEN.to_string()),
        ),
        (
            "cluster.shared-secret".to_string(),
            preseed.cluster.shared_secret.clone(),
        ),
        ("cluster.bootstrap".to_string(), bootstrap.to_string()),
        ("cluster.join-token".to_string(), String::new()),
        ("cluster.join-addresses".to_string(), peers.join(",")),
    ])
}

fn recover() -> Result<()> {
    ensure_mutable()?;
    let snap = std::env::var("SNAP").context("SNAP environment variable is missing")?;
    let status = std::process::Command::new(format!("{snap}/bin/wrapper-server"))
        .arg("recover-cluster")
        .status()
        .context("running the recovery")?;
    if !status.success() {
        bail!("recovery failed; stop the server first with: snapctl stop harvest.server");
    }
    apply(
        vec![
            ("cluster.bootstrap".to_string(), "init".to_string()),
            ("cluster.join-token".to_string(), String::new()),
            ("cluster.join-addresses".to_string(), String::new()),
        ],
        false,
    )
}

fn rotate_secrets(args: RotateArgs) -> Result<()> {
    ensure_mutable()?;
    let current = snapctl_get("cluster.shared-secret")?;
    let settings = match args.step {
        RotateStep::Stage { secret } => {
            let next = secret.unwrap_or_else(|| generate_secret(48));
            println!("{next}");
            vec![("cluster.accepted-secrets".to_string(), next)]
        }
        RotateStep::Commit { secret } => vec![
            ("cluster.shared-secret".to_string(), secret),
            ("cluster.accepted-secrets".to_string(), current),
        ],
        RotateStep::Finish => vec![("cluster.accepted-secrets".to_string(), String::new())],
    };
    apply(settings, false)
}

struct Admin {
    base: String,
    secret: String,
    http: reqwest::Client,
}

impl Admin {
    fn local() -> Result<Self> {
        let listen = non_empty_or(snapctl_get("cluster.internal-listen")?, "");
        if listen.is_empty() {
            bail!("this node is not clustered (cluster.internal-listen is unset)");
        }
        let (host, port) = listen
            .rsplit_once(':')
            .ok_or_else(|| anyhow!("cluster.internal-listen must be host:port"))?;
        let secret = match snapctl_get("cluster.shared-secret")? {
            s if !s.is_empty() => s,
            _ => token_shared_secret(&snapctl_get("cluster.join-token")?)?,
        };
        Self::at(
            &format!("http://{}:{port}", to_local_probe_host(host)),
            secret,
        )
    }

    fn at(base: &str, secret: String) -> Result<Self> {
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            secret,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()?,
        })
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .header(SECRET_HEADER, &self.secret);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("reaching the harvest server at {}", self.base))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            let reason = value["error"].as_str().unwrap_or(&text).to_string();
            bail!("{path} failed with {status}: {reason}");
        }
        Ok(value)
    }

    async fn status(&self) -> Result<Value> {
        self.call(reqwest::Method::GET, "/internal/cluster/status", None)
            .await
    }
}

async fn status(args: JsonArgs) -> Result<()> {
    let report = Admin::local()?.status().await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let membership = &report["membership"];
    println!(
        "node={} standing={} backend={} quorum={} leader={} members={}",
        report["node_id"].as_str().unwrap_or("?"),
        report["standing"].as_str().unwrap_or("?"),
        membership["backend"].as_str().unwrap_or("?"),
        membership["quorum"],
        membership["leader"].as_str().unwrap_or("-"),
        report["members"].as_array().map_or(0, Vec::len),
    );
    if let Some(servers) = report["quorium"]["servers"].as_array() {
        for server in servers {
            let role = if server["voter"] == true {
                "voter"
            } else {
                "learner"
            };
            println!(
                "server {}\t{}\t{role}",
                server["id"],
                server["address"].as_str().unwrap_or("")
            );
        }
    }
    Ok(())
}

async fn list(args: JsonArgs) -> Result<()> {
    let report = Admin::local()?.status().await?;
    let members = report["members"].as_array().cloned().unwrap_or_default();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&members)?);
        return Ok(());
    }
    for member in members {
        let draining = if member["draining"] == true {
            "\tdraining"
        } else {
            ""
        };
        println!(
            "{}\t{}{draining}",
            member["node_id"].as_str().unwrap_or(""),
            member["internal_url"].as_str().unwrap_or("")
        );
    }
    Ok(())
}

async fn remove(args: TargetArgs) -> Result<()> {
    ensure_mutable()?;
    let admin = Admin::local()?;
    admin
        .call(
            reqwest::Method::DELETE,
            &format!("/internal/cluster/servers/{}", urlencode(&args.target)),
            None,
        )
        .await?;
    println!("removed {}", args.target);
    Ok(())
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn is_instance_of(node_id: &str, name: &str) -> bool {
    node_id
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|suffix| suffix.len() == 12 && suffix.chars().all(|c| c.is_ascii_hexdigit()))
}

fn find_member<'a>(members: &'a [Value], target: &str) -> Option<&'a Value> {
    members.iter().find(|m| {
        let id = m["node_id"].as_str().unwrap_or("");
        let url = m["internal_url"].as_str().unwrap_or("");
        id == target
            || is_instance_of(id, target)
            || url.trim_end_matches('/') == target.trim_end_matches('/')
    })
}

async fn set_evacuated(args: TargetArgs, evacuated: bool) -> Result<()> {
    ensure_mutable()?;
    let local = Admin::local()?;
    let report = local.status().await?;
    let members = report["members"].as_array().cloned().unwrap_or_default();
    let member = find_member(&members, &args.target)
        .ok_or_else(|| anyhow!("no live cluster member matches {}", args.target))?;
    let url = member["internal_url"].as_str().unwrap_or_default();
    let remote = Admin::at(url, local.secret.clone())?;
    let path = if evacuated {
        "/internal/cluster/evacuate"
    } else {
        "/internal/cluster/restore"
    };
    remote.call(reqwest::Method::POST, path, None).await?;
    println!(
        "{} {}",
        if evacuated { "evacuated" } else { "restored" },
        member["node_id"].as_str().unwrap_or_default()
    );
    Ok(())
}

async fn token(args: TokenArgs) -> Result<()> {
    let admin = Admin::local()?;
    match args.command {
        TokenCommand::Create { ttl_secs, json } => {
            ensure_mutable()?;
            let created = admin
                .call(
                    reqwest::Method::POST,
                    "/internal/cluster/tokens",
                    Some(serde_json::json!({ "ttl_secs": ttl_secs.max(1) })),
                )
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&created)?);
            } else {
                println!("{}", created["token"].as_str().unwrap_or_default());
            }
        }
        TokenCommand::List(JsonArgs { json }) => {
            let report = admin.status().await?;
            let expiries = report["quorium"]["token_expiries_ms"].clone();
            if json {
                println!("{}", serde_json::to_string_pretty(&expiries)?);
            } else {
                for expiry in expiries.as_array().cloned().unwrap_or_default() {
                    println!(
                        "outstanding token expires_unix={}",
                        expiry.as_u64().unwrap_or(0) / 1000
                    );
                }
            }
        }
        TokenCommand::Revoke => {
            ensure_mutable()?;
            admin
                .call(reqwest::Method::DELETE, "/internal/cluster/tokens", None)
                .await?;
            println!("revoked every outstanding join token");
        }
    }
    Ok(())
}

fn validate_cluster_url(value: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(value).context("invalid cluster URL")?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        bail!("cluster URL must be http or https");
    }
    if parsed.host_str().is_none() {
        bail!("cluster URL must include a host");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preseed() -> Preseed {
        serde_yaml::from_str(
            r#"
cluster:
  shared_secret: "secret"
systems:
  - name: n1
    address: "http://10.0.0.1:8081"
  - name: n2
    address: "http://10.0.0.2:8081"
  - name: n3
    address: "http://10.0.0.3:8081"
"#,
        )
        .expect("preseed")
    }

    fn setting<'a>(settings: &'a Settings, key: &str) -> &'a str {
        settings
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .unwrap_or_default()
    }

    #[test]
    fn the_first_preseeded_system_initializes_the_cluster() {
        let settings = preseed_settings(&preseed(), "n1").unwrap();
        assert_eq!(setting(&settings, "cluster.bootstrap"), "init");
        assert_eq!(setting(&settings, "cluster.backend"), "quorium");
        assert_eq!(
            setting(&settings, "cluster.internal-url"),
            "http://10.0.0.1:8081"
        );
    }

    #[test]
    fn other_preseeded_systems_join_their_peers() {
        let settings = preseed_settings(&preseed(), "n2").unwrap();
        assert_eq!(setting(&settings, "cluster.bootstrap"), "join");
        assert_eq!(
            setting(&settings, "cluster.join-addresses"),
            "http://10.0.0.1:8081,http://10.0.0.3:8081"
        );
        assert_eq!(setting(&settings, "cluster.shared-secret"), "secret");
    }

    #[test]
    fn a_preseed_without_this_node_is_rejected() {
        assert!(preseed_settings(&preseed(), "n9").is_err());
    }

    #[test]
    fn the_shared_secret_is_read_from_a_join_token() {
        let token = URL_SAFE_NO_PAD.encode(r#"{"c":"id","a":[],"s":"once","k":"cluster"}"#);
        assert_eq!(token_shared_secret(&token).unwrap(), "cluster");
        let without = URL_SAFE_NO_PAD.encode(r#"{"c":"id","a":[],"s":"once"}"#);
        assert!(token_shared_secret(&without).is_err());
        assert!(token_shared_secret("garbage!").is_err());
    }

    #[test]
    fn members_match_by_id_name_or_url() {
        let members = vec![
            serde_json::json!({ "node_id": "web-1-0123456789ab", "internal_url": "http://10.0.0.1:8081" }),
        ];
        assert!(find_member(&members, "web-1-0123456789ab").is_some());
        assert!(find_member(&members, "web-1").is_some());
        assert!(find_member(&members, "http://10.0.0.1:8081/").is_some());
        assert!(find_member(&members, "web").is_none());
    }

    #[test]
    fn cluster_url_validation_rejects_invalid_schemes() {
        assert!(validate_cluster_url("ftp://example.com").is_err());
        assert!(validate_cluster_url("http://example.com:8081").is_ok());
    }

    #[test]
    fn path_segments_are_encoded() {
        assert_eq!(urlencode("http://a:1"), "http%3A%2F%2Fa%3A1");
        assert_eq!(urlencode("123"), "123");
    }
}

use std::time::Duration;

use anyhow::{bail, Result};

use super::super::images::{quote, tail};
use super::super::{CodeEnv, WORKSPACE};
use super::{base_path, Preview};
use crate::lxd::containers::Command;

const START_WAIT: Duration = Duration::from_secs(90);
const LOG_FILE: &str = "/tmp/harvest-preview.log";

pub fn port_probe(port: u16) -> String {
    format!("timeout 1 bash -c 'exec 3<>/dev/tcp/127.0.0.1/{port}'")
}

pub fn start_script(preview: &Preview, command: &str) -> String {
    let port = preview.port.map(|p| p.to_string()).unwrap_or_default();
    format!(
        "cd {WORKSPACE} && HARVEST_BASE_PATH={base} PORT={port} setsid nohup bash -lc {command} > {LOG_FILE} 2>&1 < /dev/null &",
        base = quote(&base_path(&preview.id)),
        command = quote(command),
    )
}

async fn listening(env: &CodeEnv, container: &str, port: u16) -> Result<bool> {
    let probe = env
        .lxd
        .run(container, Command::shell(&port_probe(port)).timeout(Duration::from_secs(5)))
        .await?;
    Ok(probe.succeeded())
}

pub async fn address(env: &CodeEnv, container: &str, preview: &Preview) -> Result<String> {
    let Some(port) = preview.port else {
        bail!("this preview has no port");
    };
    if !listening(env, container, port).await? {
        start(env, container, preview, port).await?;
    }
    let ip = env.lxd.wait_for_ipv4(container, Duration::from_secs(30)).await?;
    Ok(format!("{ip}:{port}"))
}

async fn start(env: &CodeEnv, container: &str, preview: &Preview, port: u16) -> Result<()> {
    let Some(command) = &preview.start_command else {
        bail!("nothing listens on port {port} and the preview has no start command");
    };
    env.lxd
        .run(container, Command::shell(&start_script(preview, command)).timeout(Duration::from_secs(15)))
        .await?;
    let deadline = tokio::time::Instant::now() + START_WAIT;
    while tokio::time::Instant::now() < deadline {
        if listening(env, container, port).await? {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let log = env
        .lxd
        .run(container, Command::shell(&format!("tail -c 4000 {LOG_FILE}")))
        .await
        .map(|o| o.stdout)
        .unwrap_or_default();
    bail!("the app did not listen on port {port} within {}s:\n{}", START_WAIT.as_secs(), tail(&log, 4_000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::previews::{Audience, Kind};

    #[test]
    fn the_start_script_runs_detached_with_the_base_path_and_port() {
        let preview = Preview {
            id: "abc".into(),
            session_id: "s".into(),
            kind: Kind::Web,
            port: Some(5173),
            start_command: None,
            audience: Audience::Owner,
            owner_id: "u".into(),
            project_name: "r".into(),
            branch: "b".into(),
            session_status: "active".into(),
            session_container: None,
        };
        let script = start_script(&preview, "npm run dev");
        assert!(script.contains("HARVEST_BASE_PATH='/previews/abc/web/' PORT=5173"));
        assert!(script.contains("setsid nohup bash -lc 'npm run dev'"));
        assert!(script.ends_with('&'));
    }
}

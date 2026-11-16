use std::time::Duration;

use anyhow::{bail, Result};

use super::super::images::{quote, tail};
use super::super::{CodeEnv, WORKSPACE};
use super::diagnosis::{self, Facts};
use super::Preview;
use crate::lxd::containers::Command;

const START_WAIT: Duration = Duration::from_secs(90);
const POLL: Duration = Duration::from_secs(2);
const LOG_FILE: &str = "/tmp/harvest-preview.log";
const PID_FILE: &str = "/tmp/harvest-preview.pid";
const LOG_CHARS: usize = 4_000;
const LOOPBACK: &str = "127.0.0.1";

pub fn port_probe(host: &str, port: u16) -> String {
    format!("timeout 1 bash -c 'exec 3<>/dev/tcp/{host}/{port}'")
}

pub fn start_script(port: u16, command: &str) -> String {
    format!(
        "HOST=0.0.0.0 PORT={port} setsid nohup bash -lc {command} > {LOG_FILE} 2>&1 < /dev/null & echo $! > {PID_FILE}",
        command = quote(command),
    )
}

pub fn stop_script() -> String {
    format!(
        "pid=$(cat {PID_FILE} 2>/dev/null) || exit 0; kill -TERM -- -\"$pid\" 2>/dev/null; \
         for i in 1 2 3 4 5; do kill -0 -- -\"$pid\" 2>/dev/null || break; sleep 1; done; \
         kill -KILL -- -\"$pid\" 2>/dev/null; rm -f {PID_FILE}; exit 0"
    )
}

pub async fn stop(env: &CodeEnv, container: &str) -> Result<()> {
    env.lxd
        .run(container, Command::shell(&stop_script()).timeout(Duration::from_secs(15)))
        .await?;
    Ok(())
}

pub fn running_probe() -> String {
    format!("kill -0 \"$(cat {PID_FILE} 2>/dev/null)\" 2>/dev/null")
}

async fn succeeds(env: &CodeEnv, container: &str, script: &str) -> Result<bool> {
    let output = env
        .lxd
        .run(container, Command::shell(script).timeout(Duration::from_secs(5)))
        .await?;
    Ok(output.succeeded())
}

async fn listening(env: &CodeEnv, container: &str, host: &str, port: u16) -> Result<bool> {
    succeeds(env, container, &port_probe(host, port)).await
}

pub async fn address(env: &CodeEnv, container: &str, preview: &Preview) -> Result<String> {
    let Some(port) = preview.port else {
        bail!("this preview has no port");
    };
    let ip = env.lxd.wait_for_ipv4(container, Duration::from_secs(30)).await?;
    if !listening(env, container, &ip, port).await? {
        start(env, container, &ip, port, preview.start_command.as_deref()).await?;
    }
    Ok(format!("{ip}:{port}"))
}

async fn start(env: &CodeEnv, container: &str, ip: &str, port: u16, command: Option<&str>) -> Result<()> {
    if listening(env, container, LOOPBACK, port).await? {
        bail!(failure(env, container, port).await);
    }
    let Some(command) = command else {
        bail!("nothing listens on port {port} and the preview has no start command");
    };
    env.lxd
        .run(
            container,
            Command::shell(&start_script(port, command)).in_dir(WORKSPACE).timeout(Duration::from_secs(15)),
        )
        .await?;
    let deadline = tokio::time::Instant::now() + START_WAIT;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(POLL).await;
        if listening(env, container, ip, port).await? {
            return Ok(());
        }
        if !succeeds(env, container, &running_probe()).await? {
            break;
        }
    }
    bail!(failure(env, container, port).await)
}

async fn failure(env: &CodeEnv, container: &str, port: u16) -> String {
    let log = env
        .lxd
        .run(container, Command::shell(&format!("tail -c {LOG_CHARS} {LOG_FILE} 2>/dev/null")))
        .await
        .map(|o| tail(&o.stdout, LOG_CHARS))
        .unwrap_or_default();
    let running = succeeds(env, container, &running_probe()).await.unwrap_or(false);
    let local_only = listening(env, container, LOOPBACK, port).await.unwrap_or(false);
    let facts = Facts { log: &log, running, local_only };
    diagnosis::diagnose(&facts).report(port, START_WAIT.as_secs(), &log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_script_runs_detached_with_host_and_port_and_records_its_pid() {
        let script = start_script(5173, "npm run dev");
        assert!(script.starts_with("HOST=0.0.0.0 PORT=5173 setsid nohup bash -lc 'npm run dev'"));
        assert!(script.ends_with(&format!("& echo $! > {PID_FILE}")));
        assert!(!script.contains("HARVEST_BASE_PATH"));
    }

    #[test]
    fn probes_target_the_given_address() {
        assert_eq!(port_probe("10.0.0.5", 80), "timeout 1 bash -c 'exec 3<>/dev/tcp/10.0.0.5/80'");
        assert!(running_probe().starts_with("kill -0"));
        let stop = stop_script();
        assert!(stop.starts_with(&format!("pid=$(cat {PID_FILE} 2>/dev/null) || exit 0;")));
        assert!(stop.contains("kill -TERM -- -\"$pid\""));
        assert!(stop.ends_with("exit 0"));
    }
}

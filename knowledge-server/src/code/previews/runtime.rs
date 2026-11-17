use std::time::Duration;

use anyhow::{bail, Result};

use super::super::images::tail;
use super::super::process::Process;
use super::super::{CodeEnv, WORKSPACE};
use super::diagnosis::{self, Facts};
use super::Preview;
use crate::lxd::containers::Command;

const START_WAIT: Duration = Duration::from_secs(90);
const POLL: Duration = Duration::from_secs(2);
const LOG_CHARS: usize = 4_000;
const LOOPBACK: &str = "127.0.0.1";

pub fn process_of(preview_id: &str) -> Process {
    Process::named(&format!("preview-{preview_id}"))
}

pub fn port_probe(host: &str, port: u16) -> String {
    format!("timeout 1 bash -c 'exec 3<>/dev/tcp/{host}/{port}'")
}

pub async fn stop(env: &CodeEnv, container: &str, preview_id: &str) -> Result<()> {
    let script = process_of(preview_id).stop_script();
    env.lxd
        .run(container, Command::shell(&script).timeout(Duration::from_secs(15)))
        .await?;
    Ok(())
}

pub async fn log(env: &CodeEnv, container: &str, preview_id: &str) -> Result<String> {
    read_log(env, container, &process_of(preview_id)).await
}

async fn read_log(env: &CodeEnv, container: &str, process: &Process) -> Result<String> {
    let output = env
        .lxd
        .run(container, Command::shell(&process.log_script(LOG_CHARS)))
        .await?;
    Ok(tail(&output.stdout, LOG_CHARS))
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
        start(env, container, &ip, port, preview).await?;
    }
    Ok(format!("{ip}:{port}"))
}

async fn start(env: &CodeEnv, container: &str, ip: &str, port: u16, preview: &Preview) -> Result<()> {
    let process = process_of(&preview.id);
    if listening(env, container, LOOPBACK, port).await? {
        bail!(failure(env, container, &process, port).await);
    }
    let Some(command) = &preview.start_command else {
        bail!("nothing listens on port {port} and the preview has no start command");
    };
    env.lxd
        .run(
            container,
            Command::shell(&process.start_script(command, &app_environment(port)))
                .in_dir(WORKSPACE)
                .timeout(Duration::from_secs(15)),
        )
        .await?;
    let deadline = tokio::time::Instant::now() + START_WAIT;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(POLL).await;
        if listening(env, container, ip, port).await? {
            return Ok(());
        }
        if !succeeds(env, container, &process.running_probe()).await? {
            break;
        }
    }
    bail!(failure(env, container, &process, port).await)
}

fn app_environment(port: u16) -> [(&'static str, String); 2] {
    [("HOST", "0.0.0.0".into()), ("PORT", port.to_string())]
}

async fn failure(env: &CodeEnv, container: &str, process: &Process, port: u16) -> String {
    let log = read_log(env, container, process).await.unwrap_or_default();
    let running = succeeds(env, container, &process.running_probe()).await.unwrap_or(false);
    let local_only = listening(env, container, LOOPBACK, port).await.unwrap_or(false);
    let facts = Facts { log: &log, running, local_only };
    diagnosis::diagnose(&facts).report(port, START_WAIT.as_secs(), &log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_run_as_supervised_processes_with_their_port() {
        let script = process_of("abc").start_script("npm run dev", &app_environment(5173));
        assert!(script.contains("HOST='0.0.0.0' PORT='5173' setsid nohup"));
        assert!(script.contains("/tmp/harvest-preview-abc.log"));
    }

    #[test]
    fn probes_target_the_given_address() {
        assert_eq!(port_probe("10.0.0.5", 80), "timeout 1 bash -c 'exec 3<>/dev/tcp/10.0.0.5/80'");
    }
}

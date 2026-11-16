use std::time::Duration;

use anyhow::{bail, Result};

use super::super::images::{quote, tail};
use super::super::{CodeEnv, WORKSPACE};
use super::diagnosis::{self, Facts};
use super::Preview;
use crate::lxd::containers::Command;

const START_WAIT: Duration = Duration::from_secs(90);
const POLL: Duration = Duration::from_secs(2);
const LOG_CHARS: usize = 4_000;
const LOOPBACK: &str = "127.0.0.1";

pub struct Files {
    log: String,
    pid: String,
}

impl Files {
    pub fn of(preview_id: &str) -> Self {
        Self {
            log: quote(&format!("/tmp/harvest-preview-{preview_id}.log")),
            pid: quote(&format!("/tmp/harvest-preview-{preview_id}.pid")),
        }
    }
}

pub fn port_probe(host: &str, port: u16) -> String {
    format!("timeout 1 bash -c 'exec 3<>/dev/tcp/{host}/{port}'")
}

pub fn start_script(files: &Files, port: u16, command: &str) -> String {
    format!(
        "HOST=0.0.0.0 PORT={port} setsid nohup bash -lc {command} > {log} 2>&1 < /dev/null & echo $! > {pid}",
        command = quote(command),
        log = files.log,
        pid = files.pid,
    )
}

pub fn stop_script(files: &Files) -> String {
    format!(
        "pid=$(cat {pid} 2>/dev/null) || exit 0; kill -TERM -- -\"$pid\" 2>/dev/null; \
         for i in 1 2 3 4 5; do kill -0 -- -\"$pid\" 2>/dev/null || break; sleep 1; done; \
         kill -KILL -- -\"$pid\" 2>/dev/null; rm -f {pid}; exit 0",
        pid = files.pid,
    )
}

pub fn running_probe(files: &Files) -> String {
    format!("kill -0 \"$(cat {} 2>/dev/null)\" 2>/dev/null", files.pid)
}

pub async fn stop(env: &CodeEnv, container: &str, preview_id: &str) -> Result<()> {
    let script = stop_script(&Files::of(preview_id));
    env.lxd
        .run(container, Command::shell(&script).timeout(Duration::from_secs(15)))
        .await?;
    Ok(())
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
    let files = Files::of(&preview.id);
    if listening(env, container, LOOPBACK, port).await? {
        bail!(failure(env, container, &files, port).await);
    }
    let Some(command) = &preview.start_command else {
        bail!("nothing listens on port {port} and the preview has no start command");
    };
    env.lxd
        .run(
            container,
            Command::shell(&start_script(&files, port, command)).in_dir(WORKSPACE).timeout(Duration::from_secs(15)),
        )
        .await?;
    let deadline = tokio::time::Instant::now() + START_WAIT;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(POLL).await;
        if listening(env, container, ip, port).await? {
            return Ok(());
        }
        if !succeeds(env, container, &running_probe(&files)).await? {
            break;
        }
    }
    bail!(failure(env, container, &files, port).await)
}

async fn failure(env: &CodeEnv, container: &str, files: &Files, port: u16) -> String {
    let log = env
        .lxd
        .run(container, Command::shell(&format!("tail -c {LOG_CHARS} {} 2>/dev/null", files.log)))
        .await
        .map(|o| tail(&o.stdout, LOG_CHARS))
        .unwrap_or_default();
    let running = succeeds(env, container, &running_probe(files)).await.unwrap_or(false);
    let local_only = listening(env, container, LOOPBACK, port).await.unwrap_or(false);
    let facts = Facts { log: &log, running, local_only };
    diagnosis::diagnose(&facts).report(port, START_WAIT.as_secs(), &log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_preview_keeps_its_own_log_and_pid_files() {
        let files = Files::of("abc");
        let script = start_script(&files, 5173, "npm run dev");
        assert!(script.starts_with("HOST=0.0.0.0 PORT=5173 setsid nohup bash -lc 'npm run dev'"));
        assert!(script.contains("> '/tmp/harvest-preview-abc.log' 2>&1"));
        assert!(script.ends_with("& echo $! > '/tmp/harvest-preview-abc.pid'"));
        assert!(running_probe(&Files::of("xyz")).contains("/tmp/harvest-preview-xyz.pid"));
    }

    #[test]
    fn stopping_ends_the_whole_process_group_of_one_preview() {
        let stop = stop_script(&Files::of("abc"));
        assert!(stop.starts_with("pid=$(cat '/tmp/harvest-preview-abc.pid' 2>/dev/null) || exit 0;"));
        assert!(stop.contains("kill -TERM -- -\"$pid\""));
        assert!(stop.ends_with("exit 0"));
    }

    #[test]
    fn probes_target_the_given_address() {
        assert_eq!(port_probe("10.0.0.5", 80), "timeout 1 bash -c 'exec 3<>/dev/tcp/10.0.0.5/80'");
    }
}

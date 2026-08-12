use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::timeout;

#[derive(Debug)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// A single line of output as it's produced, tagged with which stream it came from.
#[derive(Debug, Clone)]
pub struct OutputLine {
    pub stderr: bool,
    pub line:   String,
}

pub async fn run_command(command: &str, timeout_secs: u64) -> Result<CommandResult, String> {
    run_command_streaming(command, timeout_secs, None).await
}

/// Runs `command`, forwarding each line of stdout/stderr to `on_line` as it's produced
/// (in addition to accumulating the full output for the final result, same as `run_command`).
pub async fn run_command_streaming(
    command:      &str,
    timeout_secs: u64,
    on_line:      Option<mpsc::UnboundedSender<OutputLine>>,
) -> Result<CommandResult, String> {
    let fut = async {
        let mut child = Command::new("bash")
            .arg("-c")
            .arg(command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn failed: {e}"))?;

        let mut stdout_lines = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
        let mut stderr_lines = BufReader::new(child.stderr.take().expect("piped stderr")).lines();

        let mut stdout_buf = String::new();
        let mut stderr_buf = String::new();
        let mut stdout_open = true;
        let mut stderr_open = true;

        while stdout_open || stderr_open {
            tokio::select! {
                res = stdout_lines.next_line(), if stdout_open => {
                    match res.map_err(|e| format!("read stdout failed: {e}"))? {
                        Some(line) => {
                            stdout_buf.push_str(&line);
                            stdout_buf.push('\n');
                            if let Some(tx) = &on_line {
                                let _ = tx.send(OutputLine { stderr: false, line });
                            }
                        }
                        None => stdout_open = false,
                    }
                }
                res = stderr_lines.next_line(), if stderr_open => {
                    match res.map_err(|e| format!("read stderr failed: {e}"))? {
                        Some(line) => {
                            stderr_buf.push_str(&line);
                            stderr_buf.push('\n');
                            if let Some(tx) = &on_line {
                                let _ = tx.send(OutputLine { stderr: true, line });
                            }
                        }
                        None => stderr_open = false,
                    }
                }
            }
        }

        let status = child.wait().await.map_err(|e| format!("wait failed: {e}"))?;

        Ok(CommandResult {
            stdout:    stdout_buf,
            stderr:    stderr_buf,
            exit_code: status.code().unwrap_or(-1),
        })
    };

    timeout(Duration::from_secs(timeout_secs), fut)
        .await
        .unwrap_or_else(|_| Err(format!("command timed out after {timeout_secs}s")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn executor_runs_echo() {
        let r = run_command("echo hello", 10).await.unwrap();
        assert_eq!(r.stdout.trim(), "hello");
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test]
    async fn executor_captures_stderr() {
        let r = run_command("echo err >&2", 10).await.unwrap();
        assert_eq!(r.stderr.trim(), "err");
        assert_eq!(r.stdout, "");
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test]
    async fn executor_captures_both_streams() {
        let r = run_command("echo out; echo err >&2", 10).await.unwrap();
        assert_eq!(r.stdout.trim(), "out");
        assert_eq!(r.stderr.trim(), "err");
    }

    #[tokio::test]
    async fn executor_nonzero_exit_code() {
        let r = run_command("exit 42", 10).await.unwrap();
        assert_eq!(r.exit_code, 42);
    }

    #[tokio::test]
    async fn executor_timeout_kills_process() {
        let result = run_command("sleep 100", 1).await;
        assert!(result.is_err(), "expected timeout error");
        assert!(
            result.unwrap_err().contains("timed out"),
            "error should mention timed out"
        );
    }

    #[tokio::test]
    async fn executor_captures_multiline_output() {
        let r = run_command("printf 'a\\nb\\nc\\n'", 10).await.unwrap();
        assert_eq!(r.stdout, "a\nb\nc\n");
        assert_eq!(r.exit_code, 0);
    }
}

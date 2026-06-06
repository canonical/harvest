use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Debug)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

pub async fn run_command(command: &str, timeout_secs: u64) -> Result<CommandResult, String> {
    let fut = async {
        let output = Command::new("bash")
            .arg("-c")
            .arg(command)
            .output()
            .await
            .map_err(|e| format!("spawn failed: {e}"))?;

        Ok(CommandResult {
            stdout:    String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr:    String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(-1),
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

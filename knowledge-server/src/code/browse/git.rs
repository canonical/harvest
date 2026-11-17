use std::io::Write;
use std::path::Path;
use std::process::Output;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::Engine;
use knowledge_harvester::git::GitCredentials;
use tempfile::NamedTempFile;
use tokio::process::Command;

const GIT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

pub struct Git<'a> {
    credentials: &'a GitCredentials,
}

impl<'a> Git<'a> {
    pub fn new(credentials: &'a GitCredentials) -> Self {
        Self { credentials }
    }

    pub async fn run_in(&self, repository: &Path, args: &[&str]) -> Result<Vec<u8>> {
        self.run(&with_git_dir(repository, args)?).await
    }

    pub async fn succeeds_in(&self, repository: &Path, args: &[&str]) -> Result<bool> {
        Ok(self.output(&with_git_dir(repository, args)?).await?.status.success())
    }

    pub async fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        let output = self.output(args).await?;
        if !output.status.success() {
            bail!("git {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(output.stdout)
    }

    async fn output(&self, args: &[&str]) -> Result<Output> {
        let authentication = Authentication::for_credentials(self.credentials)?;
        tokio::time::timeout(GIT_TIMEOUT, authentication.command(args).output())
            .await
            .context("git took too long")?
            .context("running git")
    }
}

fn with_git_dir<'a>(repository: &'a Path, args: &[&'a str]) -> Result<Vec<&'a str>> {
    let mut full_args = vec!["--git-dir", path_str(repository)?];
    full_args.extend_from_slice(args);
    Ok(full_args)
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str().context("the mirror path is not valid UTF-8")
}

struct Authentication {
    environment: Vec<(&'static str, String)>,
    _private_key: Option<NamedTempFile>,
}

impl Authentication {
    fn for_credentials(credentials: &GitCredentials) -> Result<Self> {
        match credentials {
            GitCredentials::Anonymous => Ok(Self { environment: Vec::new(), _private_key: None }),
            GitCredentials::Basic { username, password } => Ok(Self {
                environment: http_header_environment(username, password),
                _private_key: None,
            }),
            GitCredentials::SshKey { private_key } => {
                let key_file = private_key_file(private_key)?;
                let ssh_command = format!(
                    "ssh -i '{}' -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new -o BatchMode=yes",
                    path_str(key_file.path())?
                );
                Ok(Self {
                    environment: vec![("GIT_SSH_COMMAND", ssh_command)],
                    _private_key: Some(key_file),
                })
            }
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .envs(self.environment.iter().map(|(name, value)| (*name, value)))
            .kill_on_drop(true);
        command
    }
}

fn http_header_environment(username: &str, password: &str) -> Vec<(&'static str, String)> {
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
    vec![
        ("GIT_CONFIG_COUNT", "1".into()),
        ("GIT_CONFIG_KEY_0", "http.extraHeader".into()),
        ("GIT_CONFIG_VALUE_0", format!("Authorization: Basic {token}")),
    ]
}

fn private_key_file(private_key: &str) -> Result<NamedTempFile> {
    let mut file = NamedTempFile::new().context("creating a key file")?;
    file.write_all(private_key.as_bytes())?;
    if !private_key.ends_with('\n') {
        file.write_all(b"\n")?;
    }
    file.flush()?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_credentials_travel_as_an_http_header_outside_the_arguments() {
        let environment = http_header_environment("user", "secret");
        assert_eq!(environment[1], ("GIT_CONFIG_KEY_0", "http.extraHeader".to_string()));
        assert_eq!(environment[2].1, "Authorization: Basic dXNlcjpzZWNyZXQ=");
    }

    #[cfg(unix)]
    #[test]
    fn private_keys_are_readable_only_by_the_server() {
        use std::os::unix::fs::PermissionsExt;
        let file = private_key_file("-----BEGIN KEY-----").unwrap();
        let mode = file.as_file().metadata().unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(std::fs::read_to_string(file.path()).unwrap().ends_with('\n'));
    }
}

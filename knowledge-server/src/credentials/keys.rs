use std::path::Path;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPair {
    pub private_key: String,
    pub public_key: String,
    pub fingerprint: String,
}

const KEY_COMMENT: &str = "harvest";

pub async fn generate() -> Result<KeyPair> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("id_ed25519");
    run(Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", KEY_COMMENT, "-f"])
        .arg(&path))
    .await?;
    let private_key = tokio::fs::read_to_string(&path).await?;
    describe(&path, private_key).await
}

pub async fn inspect(private_key: &str) -> Result<KeyPair> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("key");
    let normalized = format!("{}\n", private_key.trim());
    write_private(&path, &normalized).await?;
    describe(&path, normalized).await
}

async fn describe(path: &Path, private_key: String) -> Result<KeyPair> {
    let public_key = run(Command::new("ssh-keygen")
        .args(["-y", "-P", "", "-f"])
        .arg(path))
    .await
    .context("the private key is invalid or protected by a passphrase")?;
    let public_key = with_comment(public_key.trim());
    let pub_path = path.with_extension("pub");
    tokio::fs::write(&pub_path, format!("{public_key}\n")).await?;
    let fingerprint = run(Command::new("ssh-keygen").arg("-lf").arg(&pub_path)).await?;
    Ok(KeyPair {
        private_key,
        public_key,
        fingerprint: fingerprint
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string(),
    })
}

fn with_comment(public_key: &str) -> String {
    let mut parts = public_key.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some(kind), Some(body)) => format!("{kind} {body} {KEY_COMMENT}"),
        _ => public_key.to_string(),
    }
}

async fn write_private(path: &Path, contents: &str) -> Result<()> {
    tokio::fs::write(path, contents).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    }
    Ok(())
}

async fn run(command: &mut Command) -> Result<String> {
    let output = command.output().await.context("running ssh-keygen")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_generated_key_can_be_inspected_back_to_the_same_public_key() {
        let generated = generate().await.unwrap();
        assert!(generated.public_key.starts_with("ssh-ed25519 "));
        assert!(generated.fingerprint.starts_with("SHA256:"));
        let inspected = inspect(&generated.private_key).await.unwrap();
        assert_eq!(inspected.public_key, generated.public_key);
        assert_eq!(inspected.fingerprint, generated.fingerprint);
    }

    #[tokio::test]
    async fn text_that_is_not_a_key_is_rejected() {
        assert!(inspect("not a key").await.is_err());
    }
}

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::executor;

pub const WORKSPACE_ROOT: &str = "/var/lib/harvest-agent/terraform";

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerraformFlavor {
    Terraform,
    Terragrunt,
}

impl TerraformFlavor {
    pub fn binary(&self) -> &'static str {
        match self {
            Self::Terraform  => "terraform",
            Self::Terragrunt => "terragrunt",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerraformAction {
    Plan,
    Apply,
    Destroy,
}

pub fn workspace_dir(root: &Path, artifact_id: &str) -> PathBuf {
    root.join(artifact_id)
}

const PRESERVED_ENTRIES: &[&str] = &[
    ".terraform",
    ".terraform.lock.hcl",
    "terraform.tfstate",
    "terraform.tfstate.backup",
    ".terragrunt-cache",
];

pub fn is_preserved_path(relative_path: &Path) -> bool {
    match relative_path.components().next() {
        Some(first) => PRESERVED_ENTRIES.iter().any(|p| first.as_os_str() == *p),
        None => false,
    }
}

pub fn sync_workspace(dir: &Path, files: &BTreeMap<String, String>) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    remove_stale_entries(dir, dir, files)?;

    for (path, content) in files {
        let full_path = dir.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(full_path, content)?;
    }
    Ok(())
}

fn remove_stale_entries(root: &Path, dir: &Path, files: &BTreeMap<String, String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(root).expect("entry must live under root");

        if is_preserved_path(relative) {
            continue;
        }

        if path.is_dir() {
            let has_tracked_child = files.keys().any(|f| Path::new(f).starts_with(relative));
            if has_tracked_child {
                remove_stale_entries(root, &path, files)?;
            } else {
                std::fs::remove_dir_all(&path)?;
            }
        } else if !files.contains_key(relative.to_string_lossy().as_ref()) {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

pub fn action_subcommand(flavor: TerraformFlavor, action: TerraformAction) -> String {
    let bin = flavor.binary();
    match action {
        TerraformAction::Plan    => format!("{bin} plan -input=false -no-color"),
        TerraformAction::Apply   => format!("{bin} apply -input=false -no-color -auto-approve"),
        TerraformAction::Destroy => format!("{bin} destroy -input=false -no-color -auto-approve"),
    }
}

fn install_terraform_snippet() -> String {
    "command -v terraform >/dev/null 2>&1 || { \
curl -fsSL https://apt.releases.hashicorp.com/gpg | gpg --dearmor -o /usr/share/keyrings/hashicorp-archive-keyring.gpg && \
echo \"deb [signed-by=/usr/share/keyrings/hashicorp-archive-keyring.gpg] https://apt.releases.hashicorp.com $(lsb_release -cs) main\" > /etc/apt/sources.list.d/hashicorp.list && \
apt-get update -y && apt-get install -y terraform; }".to_string()
}

fn install_terragrunt_snippet() -> String {
    "command -v terragrunt >/dev/null 2>&1 || { \
tg_arch=$(uname -m | sed -e 's/x86_64/amd64/' -e 's/aarch64/arm64/'); \
tg_version=$(curl -fsSL https://api.github.com/repos/gruntwork-io/terragrunt/releases/latest | grep -oE '\"tag_name\": *\"[^\"]+\"' | cut -d'\"' -f4); \
curl -fsSL -o /usr/local/bin/terragrunt \"https://github.com/gruntwork-io/terragrunt/releases/download/${tg_version}/terragrunt_linux_${tg_arch}\" && \
chmod +x /usr/local/bin/terragrunt; }".to_string()
}

pub fn install_guard_snippet(flavor: TerraformFlavor) -> String {
    match flavor {
        TerraformFlavor::Terraform  => install_terraform_snippet(),
        TerraformFlavor::Terragrunt => format!("{} && {}", install_terraform_snippet(), install_terragrunt_snippet()),
    }
}

pub fn compose_command(dir: &Path, flavor: TerraformFlavor, action: TerraformAction) -> String {
    format!(
        "cd '{}' && {} && {} init -input=false -no-color && {}",
        dir.display(),
        install_guard_snippet(flavor),
        flavor.binary(),
        action_subcommand(flavor, action),
    )
}

pub async fn run_in(
    root: &Path,
    artifact_id: &str,
    flavor: TerraformFlavor,
    action: TerraformAction,
    files: &BTreeMap<String, String>,
    timeout_secs: u64,
) -> Result<executor::CommandResult, String> {
    let dir = workspace_dir(root, artifact_id);
    sync_workspace(&dir, files).map_err(|e| format!("failed to sync workspace: {e}"))?;
    let command = compose_command(&dir, flavor, action);
    executor::run_command(&command, timeout_secs).await
}

pub async fn run(
    artifact_id: &str,
    flavor: TerraformFlavor,
    action: TerraformAction,
    files: &BTreeMap<String, String>,
    timeout_secs: u64,
) -> Result<executor::CommandResult, String> {
    run_in(Path::new(WORKSPACE_ROOT), artifact_id, flavor, action, files, timeout_secs).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    #[test]
    fn is_preserved_path_matches_terraform_state_dir() {
        assert!(is_preserved_path(Path::new(".terraform/providers/registry.terraform.io")));
    }

    #[test]
    fn is_preserved_path_matches_state_files() {
        assert!(is_preserved_path(Path::new("terraform.tfstate")));
        assert!(is_preserved_path(Path::new("terraform.tfstate.backup")));
        assert!(is_preserved_path(Path::new(".terraform.lock.hcl")));
    }

    #[test]
    fn is_preserved_path_matches_terragrunt_cache() {
        assert!(is_preserved_path(Path::new(".terragrunt-cache/abc/def")));
    }

    #[test]
    fn is_preserved_path_rejects_regular_files() {
        assert!(!is_preserved_path(Path::new("main.tf")));
        assert!(!is_preserved_path(Path::new("modules/network/main.tf")));
    }

    #[test]
    fn workspace_dir_joins_root_and_artifact_id() {
        let dir = workspace_dir(Path::new("/var/lib/harvest-agent/terraform"), "artifact-1");
        assert_eq!(dir, Path::new("/var/lib/harvest-agent/terraform/artifact-1"));
    }

    #[test]
    fn sync_workspace_writes_nested_files() {
        let tmp = TempDir::new().unwrap();
        let mut files = BTreeMap::new();
        files.insert("main.tf".to_string(), "root".to_string());
        files.insert("modules/network/main.tf".to_string(), "nested".to_string());

        sync_workspace(tmp.path(), &files).unwrap();

        assert_eq!(std::fs::read_to_string(tmp.path().join("main.tf")).unwrap(), "root");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("modules/network/main.tf")).unwrap(),
            "nested"
        );
    }

    #[test]
    fn sync_workspace_removes_stale_files() {
        let tmp = TempDir::new().unwrap();
        let mut files = BTreeMap::new();
        files.insert("main.tf".to_string(), "v1".to_string());
        sync_workspace(tmp.path(), &files).unwrap();

        let mut updated = BTreeMap::new();
        updated.insert("variables.tf".to_string(), "v2".to_string());
        sync_workspace(tmp.path(), &updated).unwrap();

        assert!(!tmp.path().join("main.tf").exists());
        assert!(tmp.path().join("variables.tf").exists());
    }

    #[test]
    fn sync_workspace_removes_stale_directories_with_no_tracked_children() {
        let tmp = TempDir::new().unwrap();
        let mut files = BTreeMap::new();
        files.insert("modules/old/main.tf".to_string(), "old".to_string());
        sync_workspace(tmp.path(), &files).unwrap();
        assert!(tmp.path().join("modules/old/main.tf").exists());

        let mut updated = BTreeMap::new();
        updated.insert("main.tf".to_string(), "new".to_string());
        sync_workspace(tmp.path(), &updated).unwrap();

        assert!(!tmp.path().join("modules").exists());
    }

    #[test]
    fn sync_workspace_preserves_terraform_state_across_runs() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".terraform/providers")).unwrap();
        std::fs::write(tmp.path().join("terraform.tfstate"), "{}").unwrap();
        std::fs::write(tmp.path().join(".terraform/providers/marker"), "x").unwrap();

        let mut files = BTreeMap::new();
        files.insert("main.tf".to_string(), "content".to_string());
        sync_workspace(tmp.path(), &files).unwrap();

        assert!(tmp.path().join("terraform.tfstate").exists());
        assert!(tmp.path().join(".terraform/providers/marker").exists());
    }

    #[test]
    fn action_subcommand_builds_plan_apply_destroy() {
        assert_eq!(
            action_subcommand(TerraformFlavor::Terraform, TerraformAction::Plan),
            "terraform plan -input=false -no-color"
        );
        assert_eq!(
            action_subcommand(TerraformFlavor::Terraform, TerraformAction::Apply),
            "terraform apply -input=false -no-color -auto-approve"
        );
        assert_eq!(
            action_subcommand(TerraformFlavor::Terragrunt, TerraformAction::Destroy),
            "terragrunt destroy -input=false -no-color -auto-approve"
        );
    }

    #[test]
    fn install_guard_snippet_only_guards_terraform_for_terraform_flavor() {
        let snippet = install_guard_snippet(TerraformFlavor::Terraform);
        assert!(snippet.contains("command -v terraform"));
        assert!(!snippet.contains("command -v terragrunt"));
    }

    #[test]
    fn install_guard_snippet_guards_both_binaries_for_terragrunt_flavor() {
        let snippet = install_guard_snippet(TerraformFlavor::Terragrunt);
        assert!(snippet.contains("command -v terraform"));
        assert!(snippet.contains("command -v terragrunt"));
    }

    #[test]
    fn compose_command_orders_cd_install_init_action() {
        let dir = Path::new("/var/lib/harvest-agent/terraform/artifact-1");
        let command = compose_command(dir, TerraformFlavor::Terraform, TerraformAction::Plan);

        let cd_pos     = command.find("cd '/var/lib/harvest-agent/terraform/artifact-1'").unwrap();
        let install_pos = command.find("command -v terraform").unwrap();
        let init_pos    = command.find("terraform init").unwrap();
        let plan_pos    = command.find("terraform plan").unwrap();

        assert!(cd_pos < install_pos);
        assert!(install_pos < init_pos);
        assert!(init_pos < plan_pos);
    }

    #[tokio::test]
    async fn run_in_executes_fake_terraform_init_and_then_action() {
        let workspace_root = TempDir::new().unwrap();
        let bin_dir = TempDir::new().unwrap();
        let fake_terraform = bin_dir.path().join("terraform");
        std::fs::write(&fake_terraform, "#!/bin/sh\necho \"ran: $*\"\n").unwrap();
        std::fs::set_permissions(&fake_terraform, std::fs::Permissions::from_mode(0o755)).unwrap();

        let original_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", bin_dir.path().display(), original_path));

        let mut files = BTreeMap::new();
        files.insert("main.tf".to_string(), "resource \"local_file\" \"x\" {}".to_string());

        let result = run_in(
            workspace_root.path(),
            "artifact-1",
            TerraformFlavor::Terraform,
            TerraformAction::Plan,
            &files,
            10,
        ).await;

        std::env::set_var("PATH", original_path);

        let result = result.unwrap();
        assert!(result.stdout.contains("ran: init"), "stdout: {}", result.stdout);
        assert!(result.stdout.contains("ran: plan"), "stdout: {}", result.stdout);
        assert_eq!(result.exit_code, 0);
        assert!(workspace_root.path().join("artifact-1").join("main.tf").exists());
    }
}

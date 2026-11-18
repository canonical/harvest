pub const SETUP_REMINDER: &str = "\n\nThis command installed packages. Once the project builds and its tests run, offer to save the commands that prepared it with save_setup_script: it changes the setup of every container of this project, so the user is asked to confirm it.";

const INSTALLERS: &[&[&str]] = &[
    &["apt-get", "install"],
    &["apt", "install"],
    &["pip", "install"],
    &["pip3", "install"],
    &["-m", "pip", "install"],
    &["npm", "install"],
    &["npm", "i"],
    &["npm", "ci"],
    &["pnpm", "install"],
    &["pnpm", "add"],
    &["yarn", "install"],
    &["yarn", "add"],
    &["cargo", "install"],
    &["go", "install"],
    &["gem", "install"],
    &["bundle", "install"],
    &["snap", "install"],
    &["uv", "pip", "install"],
    &["uv", "sync"],
    &["poetry", "install"],
];

pub fn installs_packages(script: &str) -> bool {
    script
        .split(|c| matches!(c, ';' | '&' | '|' | '\n'))
        .map(|segment| segment.split_whitespace().map(base_name).collect::<Vec<_>>())
        .any(|words| !words.contains(&"--dry-run") && INSTALLERS.iter().any(|installer| contains_run(&words, installer)))
}

fn base_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

fn contains_run(words: &[&str], run: &[&str]) -> bool {
    words.windows(run.len()).any(|window| window == run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_installs_are_recognised() {
        for script in [
            "apt-get update && apt-get install -y nodejs npm",
            "sudo apt install -y python3-venv",
            "python3 -m pip install -e .[dev]",
            "cd web && npm ci",
            "/tmp/v/bin/pip install pytest freezegun",
            "pnpm add -D vitest",
        ] {
            assert!(installs_packages(script), "{script}");
        }
    }

    #[test]
    fn ordinary_commands_are_not_installs() {
        for script in [
            "npm test",
            "node --test",
            "cat package.json | grep install",
            "git log --oneline",
            "apt-get install --dry-run nodejs",
            "pip list",
        ] {
            assert!(!installs_packages(script), "{script}");
        }
    }
}

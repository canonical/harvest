#[derive(Debug, PartialEq, Eq)]
pub enum StartFailure {
    MissingCommand(String),
    LocalOnly,
    Exited,
    StillStarting,
}

pub struct Facts<'a> {
    pub log: &'a str,
    pub running: bool,
    pub local_only: bool,
}

pub fn diagnose(facts: &Facts<'_>) -> StartFailure {
    if let Some(command) = missing_command(facts.log) {
        return StartFailure::MissingCommand(command);
    }
    if facts.local_only {
        return StartFailure::LocalOnly;
    }
    if facts.running {
        StartFailure::StillStarting
    } else {
        StartFailure::Exited
    }
}

impl StartFailure {
    pub fn report(&self, port: u16, waited_secs: u64, log: &str) -> String {
        let output = match log.trim() {
            "" => "The app printed nothing.".to_string(),
            log => format!("Last output of the start command:\n{log}"),
        };
        format!("{}\n\n{output}", self.advice(port, waited_secs))
    }

    fn advice(&self, port: u16, waited_secs: u64) -> String {
        match self {
            Self::MissingCommand(command) => format!(
                "`{command}` is not installed in the container. Install it with run_command (you are root; run apt-get update first when apt cannot find a package), save the working steps with save_setup_script, then call configure_preview again."
            ),
            Self::LocalOnly => format!(
                "The app listens on 127.0.0.1:{port} only, so the preview cannot reach it. Make it listen on 0.0.0.0 (the HOST environment variable is set to 0.0.0.0), then call configure_preview again; it stops the app it started before. If you started the app yourself with run_command, stop that process first."
            ),
            Self::Exited => format!(
                "The start command stopped before anything listened on port {port}. Fix the error shown below with run_command, then call configure_preview again."
            ),
            Self::StillStarting => format!(
                "The start command is still running, but nothing listened on port {port} within {waited_secs}s. If it is installing dependencies or building, do that first with run_command so start_command only starts the app; otherwise check that the app uses port {port} (the PORT environment variable)."
            ),
        }
    }
}

fn missing_command(log: &str) -> Option<String> {
    log.lines().rev().find_map(|line| {
        let line = line.trim();
        let before = line
            .strip_suffix(": command not found")
            .or_else(|| line.strip_suffix(": not found"))
            .or_else(|| line.strip_suffix(": No such file or directory").filter(|_| line.starts_with("env: ")))?;
        let name = before.rsplit(": ").next()?;
        let name = name.trim_matches(|c| matches!(c, '\'' | '‘' | '’' | '"'));
        (!name.is_empty() && !name.contains(' ')).then(|| name.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(log: &str, running: bool, local_only: bool) -> Facts<'_> {
        Facts { log, running, local_only }
    }

    #[test]
    fn missing_commands_are_recognised_from_common_shells() {
        for (log, command) in [
            ("bash: line 1: npm: command not found", "npm"),
            ("bash: cargo: command not found", "cargo"),
            ("sh: 1: python: not found", "python"),
            ("/bin/sh: 1: bundle: not found", "bundle"),
            ("env: ‘node’: No such file or directory", "node"),
            ("env: 'ruby': No such file or directory", "ruby"),
        ] {
            assert_eq!(diagnose(&facts(log, false, false)), StartFailure::MissingCommand(command.into()), "{log}");
        }
    }

    #[test]
    fn other_missing_files_are_not_mistaken_for_missing_commands() {
        let log = "cat: config.yml: No such file or directory";
        assert_eq!(diagnose(&facts(log, false, false)), StartFailure::Exited);
    }

    #[test]
    fn an_app_bound_to_localhost_is_reported() {
        assert_eq!(diagnose(&facts("listening on 127.0.0.1:5000", true, true)), StartFailure::LocalOnly);
    }

    #[test]
    fn running_and_stopped_processes_are_told_apart() {
        assert_eq!(diagnose(&facts("npm install…", true, false)), StartFailure::StillStarting);
        assert_eq!(diagnose(&facts("Traceback: boom", false, false)), StartFailure::Exited);
    }

    #[test]
    fn the_report_gives_advice_before_the_output() {
        let report = StartFailure::MissingCommand("npm".into()).report(5173, 90, "bash: npm: command not found");
        assert!(report.starts_with("`npm` is not installed in the container. Install it with run_command"));
        assert!(report.ends_with("Last output of the start command:\nbash: npm: command not found"));
        assert!(StartFailure::Exited.report(80, 90, "  ").ends_with("The app printed nothing."));
    }
}

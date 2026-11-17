use super::images::quote;

const STATUS_MARK: &str = "@@status ";
const LOG_MARK: &str = "@@log";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    log: String,
    pid: String,
    exit: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    Exited(i64),
    Lost,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    pub log: String,
}

impl Process {
    pub fn named(name: &str) -> Self {
        let path = |extension: &str| quote(&format!("/tmp/harvest-{name}.{extension}"));
        Self {
            log: path("log"),
            pid: path("pid"),
            exit: path("exit"),
        }
    }

    pub fn start_script(&self, command: &str, env: &[(&str, String)]) -> String {
        let assignments: String = env.iter().map(|(key, value)| format!("{key}={} ", quote(value))).collect();
        let supervised = format!("bash -lc {}; echo $? > {}", quote(command), self.exit);
        format!(
            "rm -f {exit}; {assignments}setsid nohup bash -c {supervised} > {log} 2>&1 < /dev/null & echo $! > {pid}",
            exit = self.exit,
            supervised = quote(&supervised),
            log = self.log,
            pid = self.pid,
        )
    }

    pub fn stop_script(&self) -> String {
        format!(
            "pid=$(cat {pid} 2>/dev/null) || exit 0; kill -TERM -- -\"$pid\" 2>/dev/null; \
             for i in 1 2 3 4 5; do kill -0 -- -\"$pid\" 2>/dev/null || break; sleep 1; done; \
             kill -KILL -- -\"$pid\" 2>/dev/null; rm -f {pid}; exit 0",
            pid = self.pid,
        )
    }

    pub fn running_probe(&self) -> String {
        format!("kill -0 \"$(cat {} 2>/dev/null)\" 2>/dev/null", self.pid)
    }

    pub fn log_script(&self, chars: usize) -> String {
        format!("tail -c {chars} {} 2>/dev/null", self.log)
    }

    pub fn status_script(&self, chars: usize) -> String {
        format!(
            "if [ -f {exit} ]; then echo \"{STATUS_MARK}exited $(cat {exit})\"; \
             elif {running}; then echo \"{STATUS_MARK}running\"; \
             else echo \"{STATUS_MARK}lost\"; fi; echo {LOG_MARK}; {log}; true",
            exit = self.exit,
            running = self.running_probe(),
            log = self.log_script(chars),
        )
    }
}

pub fn parse_status(output: &str) -> Status {
    let (head, log) = output.split_once(&format!("{LOG_MARK}\n")).unwrap_or((output, ""));
    let state = head
        .lines()
        .find_map(|line| line.strip_prefix(STATUS_MARK))
        .map(|status| match status.trim().split_once(' ') {
            Some(("exited", code)) => code.trim().parse().map(State::Exited).unwrap_or(State::Lost),
            _ if status.trim() == "running" => State::Running,
            _ => State::Lost,
        })
        .unwrap_or(State::Lost);
    Status {
        state,
        log: log.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_process_keeps_its_own_files() {
        let process = Process::named("preview-abc");
        let script = process.start_script("npm run dev", &[("HOST", "0.0.0.0".into()), ("PORT", "5173".into())]);
        assert!(script.starts_with("rm -f '/tmp/harvest-preview-abc.exit'; HOST='0.0.0.0' PORT='5173' setsid nohup bash -c "));
        assert!(script.contains("> '/tmp/harvest-preview-abc.log' 2>&1 < /dev/null &"));
        assert!(script.ends_with("echo $! > '/tmp/harvest-preview-abc.pid'"));
    }

    #[test]
    fn the_exit_code_is_recorded_after_the_command() {
        let script = Process::named("job-1").start_script("make test", &[]);
        assert!(script.contains("bash -lc '\\''make test'\\''; echo $? > '\\''/tmp/harvest-job-1.exit'\\''"));
    }

    #[test]
    fn stopping_ends_the_whole_process_group() {
        let stop = Process::named("preview-abc").stop_script();
        assert!(stop.starts_with("pid=$(cat '/tmp/harvest-preview-abc.pid' 2>/dev/null) || exit 0;"));
        assert!(stop.contains("kill -TERM -- -\"$pid\""));
        assert!(stop.ends_with("exit 0"));
    }

    #[test]
    fn statuses_are_parsed_with_their_log() {
        assert_eq!(
            parse_status("@@status exited 2\n@@log\nFAILED test_x\n"),
            Status { state: State::Exited(2), log: "FAILED test_x\n".into() }
        );
        assert_eq!(parse_status("@@status running\n@@log\n").state, State::Running);
        assert_eq!(parse_status("@@status lost\n@@log\n").state, State::Lost);
        assert_eq!(parse_status("garbage").state, State::Lost);
    }

    #[test]
    fn the_status_script_checks_the_exit_file_before_the_pid() {
        let script = Process::named("job-1").status_script(100);
        let exit = script.find("/tmp/harvest-job-1.exit").unwrap();
        let pid = script.find("/tmp/harvest-job-1.pid").unwrap();
        assert!(exit < pid);
        assert!(script.contains("tail -c 100 '/tmp/harvest-job-1.log'"));
    }
}

use super::model::TerminalId;
use crate::code::images::quote;
use crate::code::WORKSPACE;

const CONFIG_PATH: &str = "/etc/harvest/tmux.conf";
const CONFIG: &str = "set -g status off
set -g mouse on
set -g history-limit 50000
set -g default-terminal tmux-256color
set -as terminal-features ',xterm-256color:RGB'
set -g set-clipboard on
set -g window-size latest
set -g escape-time 0
set -g set-titles on
set -g set-titles-string '#{pane_current_command}'
";

pub fn session_name(id: &TerminalId) -> String {
    format!("harvest-{}", id.as_str())
}

pub fn attach_command(id: &TerminalId) -> Vec<String> {
    ["tmux", "-f", CONFIG_PATH, "new-session", "-A", "-s", &session_name(id), "-c", WORKSPACE]
        .map(String::from)
        .to_vec()
}

pub fn end_script(id: &TerminalId) -> String {
    format!("tmux kill-session -t {} 2>/dev/null || true", quote(&session_name(id)))
}

pub fn prepare_script() -> String {
    format!(
        "command -v tmux >/dev/null 2>&1 || {{ apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq tmux >/dev/null; }}\n\
         mkdir -p /etc/harvest\n\
         cat > {CONFIG_PATH} <<'HARVEST_TMUX'\n{CONFIG}HARVEST_TMUX\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> TerminalId {
        TerminalId::parse("0123456789abcdef0123456789abcdef").unwrap()
    }

    #[test]
    fn attaching_creates_the_session_or_joins_it_in_the_workspace() {
        assert_eq!(
            attach_command(&id()),
            [
                "tmux", "-f", "/etc/harvest/tmux.conf", "new-session", "-A", "-s",
                "harvest-0123456789abcdef0123456789abcdef", "-c", "/workspace",
            ]
        );
    }

    #[test]
    fn ending_a_terminal_is_harmless_when_it_is_already_gone() {
        assert_eq!(end_script(&id()), "tmux kill-session -t 'harvest-0123456789abcdef0123456789abcdef' 2>/dev/null || true");
    }

    #[test]
    fn the_prepared_configuration_keeps_colour_scrollback_and_the_latest_size() {
        let script = prepare_script();
        assert!(script.starts_with("command -v tmux >/dev/null 2>&1 ||"));
        for setting in ["history-limit 50000", "window-size latest", "xterm-256color:RGB", "set-clipboard on"] {
            assert!(script.contains(setting), "{setting}");
        }
        assert!(script.ends_with("HARVEST_TMUX\n"));
    }
}

pub struct Role {
    pub name: &'static str,
    pub instructions: &'static str,
    pub tools: &'static [&'static str],
    pub max_iterations: usize,
}

impl Role {
    pub fn allows(&self, tool: &str) -> bool {
        self.tools.contains(&tool)
    }
}

pub const REVIEWER: Role = Role {
    name: "reviewer",
    instructions: "## Your role: reviewer

You review changes another agent just made in this coding session. You cannot change files, run commands or commit: \
read, search and inspect only. Start with show_changes, then read the files involved and the code around them.

Judge the change against the user's request and the repository's conventions. Look for: bugs and unhandled cases, \
files the change needs but does not include or register (for example new modules a server, build or manifest must \
know about), tests that are missing or that would pass without the change, documentation or changelog entries the \
repository keeps up to date, and anything that contradicts the request. If a web app is running, check_page can \
confirm the user interface works.

Report only real problems, most serious first, each with the file and line and why it matters, in a few short \
bullets. Do not praise the change or restate it. If you find nothing worth fixing, say so in one sentence.",
    tools: &[
        "show_changes",
        "read_file",
        "search_workspace",
        "list_files",
        "check_page",
        "search_code",
        "read_sources",
        "search_symbols",
        "get_symbol_source",
        "get_file_symbols",
        "find_callers",
        "find_callees",
        "get_imports",
    ],
    max_iterations: 12,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reviewer_reads_but_cannot_change_anything() {
        for tool in ["show_changes", "read_file", "search_workspace", "check_page"] {
            assert!(REVIEWER.allows(tool), "{tool}");
        }
        for tool in ["run_command", "write_file", "replace_in_file", "propose_change", "save_setup_script", "configure_preview"] {
            assert!(!REVIEWER.allows(tool), "{tool}");
        }
    }
}

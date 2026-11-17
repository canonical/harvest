# Checks, reviews and background jobs

**Goal:** know that every change in a coding session passes the repository's checks, and get an independent review of it.

## Checks

After each agent turn that changed files, Harvest runs the session's **definition of done** on a copy of the workspace, as a background job. The header shows **Checks running…**, **Checks passed** or **Checks failed**; select it to open the **Jobs** tab.

The commands come from, in order:

1. `.harvest/checks` in the repository, one command per line (`#` starts a comment);
2. **Checks** on the project page;
3. the commands Harvest detects: test commands (`npm test`, `python3 -m pytest`, `cargo test`, `go test ./...`, `make test`) and checks (`pre-commit run --all-files`, `ruff`, `mypy`, `npm run lint`, `cargo clippy`, `go vet`, `make lint`).

When checks fail, the agent sees the output at the start of its next turn and fixes what failed. You can also select **Ask the agent to fix this** in the Jobs tab, or **Run checks** at any time.

## Reviews

After each turn that added commits, a reviewer reads the change in a fresh context: same model, read-only tools, no access to the author's reasoning. Its findings appear in the chat as **Review the latest changes**, and the author agent reads them in its next turn. Turn reviews off with `[code] review_changes = false`.

## Background jobs

The **Jobs** tab lists everything that runs in the background: the environment preparation at session start, commands the agent started with `background: true`, checks and reviews. Each shows its status, duration and output; running commands can be stopped.

# Session and preview lifecycles

## Code project image

| State | Meaning |
|---|---|
| Being prepared | building: base image, clone, setup script (≤ 30 min) |
| Ready | sessions start from this image |
| The last setup failed | see the log on the project page; fix the setup script |

The image is rebuilt when the setup script changes. Warm containers (`pool_size`, default 2) are kept for projects used in the last `warm_days` (default 7).

## Session

```text
starting ──▶ active ──▶ closed (merged | idle | manual)
    └──────────────▶ failed
```

| State | Meaning |
|---|---|
| `starting` | container being claimed or created, branch checked out |
| `active` | usable |
| `closed` | container deleted; reason `merged` (change request merged), `idle` (unused for `idle_hours`, default 24) or `manual` (ended by the user) |
| `failed` | the container could not start; open a new session |

One open session per user, project and branch.

## Preview

| Event | Effect |
|---|---|
| Created | app started; the agent or panel reports whether it answers |
| Opened | `last_used_at` updated (at most once a minute) |
| Restarted | app stopped and started again |
| Unused for `preview_ttl_hours` (24) | deleted |
| Session closed | deleted; links answer *This preview ended with its session.* |

## Session jobs

| Kind | Started | Runs |
|---|---|---|
| `prepare` | when the session becomes active | installs what the repository's files call for (Python packages and tools, npm dependencies, Rust, Go, make) |
| `command` | by the agent, `run_command` with `background: true` | the command |
| `checks` | after each turn that changed files, or **Run checks** | the definition of done, on a copy of the workspace |
| `review` | after each turn that added commits (`review_changes`) | the reviewer agent; its findings are posted in the chat |

States: `running`, then `succeeded`, `failed` or `cancelled`. Jobs run under a supervisor in the container that records their exit code, so a server restart does not lose them; the keeper picks running jobs up again every 30 seconds. A review left running by a restart is marked failed after 30 minutes.

## Code graph of a project

Every `reindex_hours` (default 6) Harvest fetches the default branch. If its commit moved, it indexes the new commit as a new version and carries over summaries and embeddings of unchanged symbols.

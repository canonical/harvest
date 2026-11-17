# Agent tools

The agent's tools depend on the mode.

| Mode | Toolsets |
|---|---|
| Chat | Knowledge, Artifacts, Skills |
| Code | Knowledge, Workspace |
| Deploy | Knowledge, Artifacts, Skills, Deploy agents, Terraform, Sandboxes |

## Knowledge

| Tool | Does |
|---|---|
| `list_repositories` | repositories and versions the user can access |
| `search_symbols` | find functions, classes and files by name or text |
| `read_sources` | read files or line ranges |
| `get_symbol_source` | a symbol's code |
| `get_file_symbols` | symbols defined in a file |
| `find_callers` / `find_callees` | call graph |
| `get_imports` | a file's imports |
| `find_subclasses` | inheritance |
| `compare_symbol_across_versions` | how a symbol differs between versions |
| `get_capability_matrix`, `get_evidence_pack` | structured comparisons with evidence |
| `run_sql` | SQL over the graph in a read-only transaction (10 s limit); only when the user can access every repository |

With `search.engine = "tiered"`, the agent gets `list_repositories`, `search_code`, `read_sources` and `compare_symbol_across_versions` instead.

In Code sessions, graph results for files changed in the workspace carry a note saying so; the agent reads those files from the workspace.

## Workspace (Code)

| Tool | Does |
|---|---|
| `run_command` | run a shell command in the session container, as root, in `/workspace`. With `background: true` it starts a job and returns its id at once. After a successful package install it reminds the agent to save a setup script, and Harvest remembers the command for the project |
| `job_output` | status and latest output of a background job |
| `wait_job` | wait up to `timeout_seconds` (default 120, at most 600) for a background job, then report it |
| `read_file`, `list_files` | read the workspace |
| `search_workspace` | `git grep` in the workspace, including uncommitted changes |
| `write_file`, `replace_in_file` | edit files |
| `show_changes` | diff against the base branch |
| `save_setup_script` | save the project's container setup script |
| `configure_preview` | create or update a preview (`kind`, `port`, `start_command`, `new`, `preview_id`) and report whether it started |
| `check_page` | open a page of the running app in headless Chromium inside the container, optionally `click`, `fill`, `press` and `wait` first, and report title, visible text, console errors and failed requests. The first call installs the browser |
| `propose_change` | open or update the change request |

## Artifacts and skills

| Tool | Does |
|---|---|
| `generate_artifact` | create or update an artifact |
| `list_skills`, `load_skill` | discover and read skills |

## Deploy

| Toolset | Tools |
|---|---|
| Deploy agents | `list_agents`, `create_lxd_agent`, `delete_agent`, `run_command`, `list_port_forwards`, `create_port_forward`, `update_port_forward`, `delete_port_forward` |
| Terraform | `run_terraform_plan`, `run_terraform_apply`, `run_terraform_destroy` |
| Sandboxes | `collocate_create_session`, `collocate_delete_session`, `collocate_exec`, `collocate_run`, `collocate_transfer_file`, `collocate_list_containers` |
| Design and provisioning | `read_provision_bundle`, `set_execution_plan`, `link_deployment_artifact`, `update_product_template`, `ask_user`, `propose_parallel_research` |

Finished background jobs are also reported at the end of the agent's next tool result, and at the start of its next turn.

The **reviewer** is the same agent with a different role: it gets `show_changes`, `read_file`, `search_workspace`, `list_files`, `check_page` and the read-only Knowledge tools, at most 12 steps, and cannot change files or run commands.

These ask the user to confirm (`confirm_action`) before they run: `create_lxd_agent`, `delete_agent`, `create_port_forward`, `update_port_forward`, `delete_port_forward`, `run_terraform_apply`, `run_terraform_destroy`.

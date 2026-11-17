# Database and migrations

Harvest stores all state in one PostgreSQL database. Extensions: `vector` (semantic search, optional) and `pg_trgm` (name search).

Migrations live in `harvest-db/migrations/` and run in order on server start (`database.migrate_on_start`). Run them manually with `knowledge-server migrate`; print the current version with `knowledge-server schema-version`. The schema is at version 26.

## Main tables

| Area | Tables |
|---|---|
| Code graph | `repositories`, `versions`, `files`, `symbols`, `imports`, `symbol_edges`, `code_units`, `unit_edges` |
| Search | `symbol_enrichments`, `evidence_cache`, `search_traces`, `search_calibrations`, `work_queue` |
| Ingestion | `ingestion_jobs`, `ingestion_progress` |
| Users and access | `users`, `groups`, `user_groups`, `user_llm_keys`, `user_api_keys`, `ssh_keys`, `platform_accounts`, `repository_access`, `auth_ephemeral` |
| Chat | `conversations`, `conversation_messages`, `chat_layouts`, `artifacts`, `skills`, `api_chat_threads`, `turn_requests`, `active_turns`, `paused_turns` |
| Code mode | `code_projects`, `code_images`, `code_containers`, `code_sessions`, `code_previews`, `code_preview_viewers`, `code_change_requests`, `lxd_identity` |
| Deploy mode | `projects`, `deploy_project_members`, `product_templates`, `deployments`, `deployment_context_artifacts`, `deployment_runs`, `execution_steps`, `execution_step_deps`, `proposals`, `design_pdf_cache` |
| Machines | `machines`, `agent_connections`, `lxd_pending_instances`, `port_forwards`, `collocate_sessions` |
| Cluster | `cluster_nodes`, `cluster_leases`, `fencing_tokens`, `project_presence` |
| Costs | `llm_calls` |

Secrets in `user_llm_keys`, `ssh_keys` and `platform_accounts` are encrypted with `security.user_key_encryption_key`.

# Harvest documentation

Harvest answers questions about your code, builds features in isolated coding sessions, and plans and runs deployments. It has three modes, switched at the top of the navigation: **Chat**, **Code** and **Deploy**.

This documentation follows [Diátaxis](https://diataxis.fr/): tutorials teach, how-to guides solve a task, reference lists facts, explanation gives the reasons.

## Tutorials

Start here if Harvest is new to you. Each one builds on the previous.

1. [Get started with Harvest](tutorials/get-started.md)
2. [Ask questions about a codebase](tutorials/ask-questions-about-a-codebase.md)
3. [Build a feature with a coding session](tutorials/build-a-feature-with-a-coding-session.md)
4. [Plan a deployment from requirements to design](tutorials/plan-a-deployment.md)
5. [Deploy to your own machines](tutorials/deploy-to-your-machines.md)

## How-to guides

**Installation and operations**
- [Install Harvest from the snap](how-to/install-from-snap.md)
- [Configure Harvest](how-to/configure-harvest.md)
- [Prepare PostgreSQL](how-to/prepare-postgresql.md)
- [Add LLM providers and choose models](how-to/add-llm-providers.md)
- [Let users bring their own API keys](how-to/user-provided-keys.md)
- [Enable Code mode with LXD](how-to/enable-code-mode.md)
- [Serve web previews on their own domain](how-to/serve-previews.md)
- [Use previews without a domain](how-to/previews-without-a-domain.md)
- [Put Harvest behind HTTPS](how-to/https-reverse-proxy.md)
- [Run Harvest as a cluster](how-to/run-a-cluster.md)
- [Back up and restore Harvest](how-to/back-up-and-restore.md)
- [Recover LXD after it loses its configuration](how-to/recover-lxd.md)
- [Upgrade Harvest](how-to/upgrade.md)
- [Read logs and change verbosity](how-to/logs.md)

**Users and access**
- [Sign users in with Google or OIDC](how-to/single-sign-on.md)
- [Manage users and roles](how-to/manage-users.md)
- [Control who sees which repositories](how-to/repository-access.md)
- [Create API keys for applications](how-to/api-keys.md)

**Repositories**
- [Index a repository](how-to/index-a-repository.md)
- [Index private repositories](how-to/index-private-repositories.md)
- [Re-index or delete a version](how-to/reindex-or-delete.md)
- [Ingest repositories from the command line](how-to/ingest-from-the-command-line.md)
- [Improve search with enrichment and embeddings](how-to/enrichment-and-embeddings.md)

**Chat**
- [Ask about a specific repository or version](how-to/scope-a-question.md)
- [Use tabs, split panes and saved layouts](how-to/chat-layouts.md)
- [Attach images and PDFs](how-to/attach-files.md)
- [Keep answers as artifacts](how-to/artifacts.md)
- [Write a skill](how-to/write-a-skill.md)

**Code**
- [Open a repository as a code project](how-to/open-a-code-project.md)
- [Set up project containers](how-to/set-up-containers.md)
- [Work with several chats and tabs in a session](how-to/session-workspace.md)
- [Preview a web app, an API or a command-line tool](how-to/preview-your-work.md)
- [Share a preview](how-to/share-a-preview.md)
- [Fix a preview that does not start](how-to/fix-a-preview.md)
- [Checks, reviews and background jobs](how-to/checks-and-reviews.md)
- [Connect GitHub, GitLab, Gitea, Gerrit or Launchpad](how-to/connect-git-platforms.md)
- [Open and update a change request](how-to/change-requests.md)
- [End a session](how-to/end-a-session.md)

**Deploy**
- [Create a project and invite members](how-to/create-a-project.md)
- [Create product templates](how-to/product-templates.md)
- [Generate and revise a design document](how-to/design-documents.md)
- [Connect a machine agent](how-to/connect-an-agent.md)
- [Let Harvest create agents in LXD](how-to/lxd-agents.md)
- [Generate and run a deployment](how-to/run-a-deployment.md)
- [Use the agent console and port forwarding](how-to/console-and-port-forwarding.md)
- [Track spending](how-to/track-spending.md)
- [Benchmark coding sessions](how-to/benchmark-coding-sessions.md)

**Integration**
- [Call Harvest from your application](how-to/call-the-chat-api.md)
- [Use Harvest from a terminal](how-to/terminal-client.md)

## Reference

- [Configuration file (`server.toml`)](reference/configuration.md)
- [Snap settings and the `harvest` CLI](reference/snap-and-cli.md)
- [HTTP API](reference/http-api.md)
- [Streaming events](reference/streaming-events.md)
- [Agent tools](reference/agent-tools.md)
- [Previews](reference/previews.md)
- [Session and preview lifecycles](reference/lifecycles.md)
- [Change request platforms](reference/change-request-platforms.md)
- [Roles and permissions](reference/permissions.md)
- [Keyboard shortcuts](reference/keyboard-shortcuts.md)
- [Harvester CLI](reference/harvester-cli.md)
- [Supported languages](reference/languages.md)
- [Database and migrations](reference/database.md)
- [Error messages](reference/errors.md)

## Explanation

- [What Harvest is](explanation/overview.md)
- [How Harvest understands code](explanation/code-understanding.md)
- [How the agent works](explanation/agent.md)
- [Coding sessions](explanation/coding-sessions.md)
- [Keeping the code graph fresh](explanation/graph-freshness.md)
- [How previews stay isolated](explanation/preview-isolation.md)
- [Deploy mode](explanation/deploy-mode.md)
- [Machine agents](explanation/machine-agents.md)
- [Security model](explanation/security.md)
- [Clustering](explanation/clustering.md)
- [Costs](explanation/costs.md)
- [Architecture](explanation/architecture.md)

Contributor documentation lives in [`developer/`](developer/).

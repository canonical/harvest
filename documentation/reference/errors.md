# Error messages

## Code mode

| Message | Cause | Fix |
|---|---|---|
| Code mode needs LXD; ask an administrator to configure it | no `[lxd]` section | [Enable Code mode](../how-to/enable-code-mode.md) |
| LXD request failed (403 Forbidden) | LXD does not trust Harvest's certificate (often after LXD lost its database) | [Recover LXD](../how-to/recover-lxd.md) |
| your credentials cannot read this repository | no credentials, or they lack access | add an SSH key or platform account in **Settings → Git access** |
| the repository URL is not a git remote | URL is not a repository | use the clone URL |
| the repository has no branches | empty repository | push a branch first |
| credentials are not configured on this server | no `security.user_key_encryption_key` | set it ([configuration](configuration.md#security)) |
| Harvest does not know which platform hosts *host*; add an account for it in Settings → Git access | self-hosted git server | add a platform account for that host |
| add your *platform* account for *host* in Settings → Git access to open change requests | no account for the host | add it |
| commit or discard the uncommitted changes first | dirty workspace when proposing | ask the agent to commit |
| there are no commits to propose yet | branch equals its base | commit work first |
| The session could not start | the container failed to start | open a new session; check the project's **Container setup** |

## Previews

| Message | Cause |
|---|---|
| The app could not start | see the diagnosis: [Fix a preview](../how-to/fix-a-preview.md) |
| Port *N* is already used by another preview of this session | each preview needs its own port |
| previews need LXD | no `[lxd]` section |
| This preview ended with its session. | session closed |
| this preview does not exist or is not shared with you | deleted, expired, or the audience excludes you |
| Starting your copy of the app… / your copy of the app is still starting | a viewer copy is being created; retry in a few seconds |

## Deploy projects

| Message | Cause |
|---|---|
| your role in this project does not allow this (*role* required) | role too low: see [permissions](permissions.md) |
| no Harvest user has this email | invite an existing user |
| a project needs at least one owner; make someone else an owner first | removing or demoting the last owner |

## Server start

| Message | Fix |
|---|---|
| security.user_key_encryption_key must be configured when any provider has user_provided_key = true | set the key |
| user_key_encryption_key must be exactly 32 bytes (64 hex characters) | regenerate with `openssl rand -hex 32` |
| semantic search is enabled but no gemini provider is configured for embeddings | add a Gemini provider with a key, or disable `[semantic]` |

# HTTP API

Base URL: the server (`http://host:8080`) or the web UI's proxy (`https://harvest.example.com/api`). Bodies are JSON.

**Authentication**

| Caller | How |
|---|---|
| Web UI and `harvest-tui` | session token from `/auth/login` or SSO, sent as a cookie or `Authorization: Bearer <jwt>` |
| Applications | API key: `Authorization: Bearer hk_live_…`, only for `/v1/chat` |
| Machine agents | per-agent token on `/agent/*` |

Errors return a status code and `{"error": "<message>"}`. See [Error messages](errors.md).

## Public

| Method | Path | Does |
|---|---|---|
| GET | `/health`, `/health/ready` | liveness; readiness (database, cluster quorum) |
| GET | `/version`, `/metrics` | build version; Prometheus metrics |
| GET | `/auth/config` | enabled sign-in methods |
| POST | `/auth/register`, `/auth/login`, `/auth/logout` | local accounts |
| GET | `/auth/google`, `/auth/oidc` (+ `/callback`) | single sign-on |
| POST / GET | `/auth/tui/login`, `/auth/tui/poll/:uuid` | terminal sign-in |
| GET | `/previews/:id` and `/reset`, `/request`, `/exec`, `/enter` | shared previews ([Previews](previews.md)) |

## Application API

| Method | Path | Does |
|---|---|---|
| POST | `/v1/chat` | ask a question ([fields](../how-to/call-the-chat-api.md)) |
| GET | `/v1/chat/:tracking_id?limit=200` | read a conversation |

## Signed-in user

| Area | Paths |
|---|---|
| Questions | `POST /query`, `POST /query/stream` (SSE, [events](streaming-events.md)), `GET /tool-description` |
| Account | `GET PATCH /auth/me`, `POST /auth/tui/authorize/:uuid` |
| Model keys | `GET /llm/providers`, `GET /llm/user-keys`, `PUT DELETE /llm/user-keys/:provider_id` |
| Git access | `GET /me/credentials`, `PUT DELETE /me/ssh-key`, `GET POST /me/accounts`, `PUT DELETE /me/accounts/:host` |
| API keys | `GET POST /api-keys`, `PATCH DELETE /api-keys/:id` |
| Conversations | `/conversations`, `/chat-layouts`, `/chat-layouts/current`, `/chat-layouts/:id` |
| Personal artifacts and skills | `/me/artifacts`, `/me/skills`, `/skills`, `/artifacts/:id`, `/artifacts/:id/download` |
| Repositories | `GET POST /repositories`, `POST /repositories/verify`, `/repositories/:name/progress`, `/stats`, `/versions`, `POST /repositories/:name/versions/:version/resync` |
| Code graph | `GET /graph/:repo/:version`, `GET /graph/:repo/:version/source` |
| Documentation | `GET /docs` |

## Code mode

| Method | Path | Does |
|---|---|---|
| GET POST | `/code/projects` | list, open a repository |
| GET | `/code/projects/:id` | project, index and image state |
| PUT | `/code/projects/:id/setup` | save the setup script |
| GET POST | `/code/projects/:id/sessions` | list, start a session (`branch`, `base`) |
| GET DELETE | `/code/sessions/:id` | session; end it |
| GET | `/code/sessions/:id/changes[?commit=<hash>]` | commits, and the diff against the base branch or of one commit |
| POST | `/code/sessions/:id/change-request` | open or update the change request |
| GET POST | `/code/sessions/:id/chats` | list, create chats |
| PATCH DELETE | `/code/sessions/:id/chats/:chat` | rename, delete |
| GET PUT | `/code/sessions/:id/layout` | workspace layout (≤ 64 KiB) |
| GET POST | `/code/sessions/:id/previews` | list, create (`kind` web\|api\|cli, `port`, `start_command`) |
| POST | `/code/sessions/:id/query/stream` | message the agent (SSE) |
| DELETE | `/code/previews/:id` | delete a preview |
| PUT | `/code/previews/:id/share` | `audience`: owner\|repository\|link |
| POST | `/code/previews/:id/restart`, `/keep` | restart; reset expiry |
| GET | `/code/previews/:id/log` | start log |
| GET | `/code/sessions/:id/jobs` | the session's jobs, newest first |
| POST | `/code/sessions/:id/checks` | run the checks now |
| GET | `/code/jobs/:id` | one job, with its latest output |
| POST | `/code/jobs/:id/stop` | stop a running job |
| PUT | `/code/projects/:id/checks` | the project's checks, one command per line (`checks`); empty uses the detected ones |

## Deploy mode

All under `/projects/:pid`, checked against the caller's [project role](permissions.md).

| Area | Paths |
|---|---|
| Project | `GET POST /projects`, `GET PATCH DELETE /projects/:pid`, `/members` |
| Chat | `/conversations`, `POST /query`, `/query/stream`, `GET /events` (SSE), `POST /confirm-action`, `/resume` |
| Content | `/artifacts`, `/skills`, `/templates` (`/upload`, `/:tid`, `/:tid/download`) |
| Deployments | `/deployments`, `/deployments/:did` with `/deploy`, `/redeploy`, `/destroy`, `/runs`, `/environment/questions`, `/execution-plan`, `/run-dag`, `/run-destroy-dag`, `/context-artifacts`, `/proposals/:id/approve\|discard` |
| Design | `/deployments/:did/design/generate[/stream]`, `/decisions`, `/revise`, `/propose-change/stream`, `/pdf`, `/content` |
| Provisioning | `/deployments/:did/provision/generate[/stream]`, `/propose-change[/stream]`, `/apply-change` |
| Costs | `/cost`, `/cost/by-model`, `/cost/by-user`, `/conversations/:cid/cost`, `/deployments/:did/cost[/calls]` |
| Agents | `/agents`, `/agents/:id` (`DELETE`, `/execute`, `/terraform`, `/console`, `/start`, `/stop`, `/restart`, `/port-forwards`), `/agents/rotate-install-token`, `/agents/flavors`, `/agents/lxd` |

Machine agents use `GET /agents/:project_id/install.sh`, `GET /agents/binary/harvest-agent`, and `/agent/events|results|output|ping|console|tunnel`. Port forwards are served at `/agents/:agent_id/:route/*`.

## Administrators

`/admin/users`, `/admin/users/:id/role`, `/admin/conversation-moves`, `/admin/skills`, `/admin/repositories`.

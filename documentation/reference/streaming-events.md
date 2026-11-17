# Streaming events

`/query/stream`, `/code/sessions/:id/query/stream` and `/projects/:pid/query/stream` answer with server-sent events. Each `data:` line is a JSON object with a `type`.

| `type` | Fields | Meaning |
|---|---|---|
| `intent` | `intent` | what the agent understood |
| `phase` | `phase` | current stage, e.g. searching, reading |
| `thinking` / `thinking_delta` | `text` | model reasoning, whole or incremental |
| `text_delta` | `text` | the answer, incremental |
| `tool_call` | `name`, `input` | a tool is called |
| `tool_result` | `name`, `preview`, `view` | its result, shortened; `view` is a structured rendering when available |
| `question` | `question`, `choices` | the agent asks the user; reply with the next message |
| `confirm_action` | action details | Deploy: an action needs approval (`POST /projects/:pid/confirm-action`) |
| `title_updated` | `title` | conversation renamed |
| `parallel_research_started`, `parallel_research_lead_done`, `parallel_research_merge_started` | — | Deploy: parallel research progress |
| `enumeration` | — | the agent is listing many items |
| `done` | `answer`, `sources`, `tool_calls_made`, `tool_errors`, `turns`, `provider_used`, `duration_ms`, `hit_max_iterations`, `usage`, `llm_call_count` | final answer |
| `error` | `message` | the turn failed |

`hit_max_iterations: true` means the agent stopped at `agent.max_iterations` and the answer may be incomplete.

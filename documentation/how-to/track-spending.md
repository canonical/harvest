# Track spending

**Goal:** know what model usage costs.

Harvest records one row per model call in `llm_calls`, with its position in the turn (`iteration_index`), tokens (fresh input, cache reads, cache writes, output, reasoning) and cost. Prices come from each provider's `pricing` (micro-dollars per 1,000 tokens) or the built-in table. Set prices for models Harvest does not know:

```toml
[[llm]]
id = "openrouter"
# ...
[llm.pricing]
input_per_1k       = 3000     # $3 per million input tokens
output_per_1k      = 15000
cache_read_per_1k  = 300
cache_creation_per_1k = 3750
```

Read the totals:

| Scope | Endpoint |
|---|---|
| Project | `GET /projects/<id>/cost`, `/cost/by-model`, `/cost/by-user` |
| Conversation | `GET /projects/<id>/conversations/<cid>/cost` |
| Deployment | `GET /projects/<id>/deployments/<did>/cost`, `/cost/calls` |
| One API call | `cost_microusd` in the `/v1/chat` response |

To cap usage, limit concurrent turns (`agent.max_active_turns`, `max_active_turns_per_user`) and the agent's steps per turn (`agent.max_iterations`). With [user-provided keys](user-provided-keys.md), each user pays their own provider.

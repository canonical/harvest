# Call Harvest from your application

**Goal:** ask Harvest questions from code, with the same agent as the web UI.

Create an [API key](api-keys.md), then:

```bash
curl -s https://harvest.example.com/v1/chat \
  -H "Authorization: Bearer $HARVEST_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
        "message": "Where does itsdangerous check token expiry?",
        "tracking_id": "ticket-4521",
        "sources": "url",
        "diagrams": "disabled"
      }'
```

| Field | Default | Meaning |
|---|---|---|
| `message` | required | the question |
| `tracking_id` | new | reuse it to continue the same conversation |
| `sources` | `url` | add a `sources` array: `url` as links to the files on their git host, `raw` as source records (repository, file, lines), `disabled` for none |
| `diagrams` | `ascii` | `mermaid`, `ascii` or `disabled` |
| `include_history` | `false` | return the conversation's earlier messages |
| `provider_id`, `model` | server default | choose the model |

The response contains `tracking_id`, `conversation_id`, `response` (Markdown), `usage`, `tool_calls_made`, `tool_errors`, `turns`, `duration_ms`, `llm_call_count`, `cost_microusd`, `provider`, and `sources` and `history` when asked for.

Read a conversation back: `GET /v1/chat/<tracking_id>?limit=200`.

Calls are synchronous; a long question can take a minute or more, so allow a long client timeout. Full API: [HTTP API](../reference/http-api.md).

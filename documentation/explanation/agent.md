# How the agent works

Each message starts a **turn**: the agent calls the model, the model asks for tools, Harvest runs them and returns results, until the model writes an answer.

```text
message ─▶ model ─▶ tool calls ─▶ results ─▶ model ─▶ … ─▶ answer
```

## Tools by mode

The mode decides what the agent can touch: Chat can read code and write artifacts; Code can also edit, run and preview in the session container; Deploy can also drive machine agents, Terraform and sandboxes. [Full list](../reference/agent-tools.md).

A tool can only see what the user can see: repository access is checked on every call, not only when the conversation starts.

## Limits

- **Steps:** at most `agent.max_iterations` model calls per turn (default 20). If reached, the answer says so.
- **Context:** when a conversation passes `compaction_threshold_chars`, older messages are summarised and the last `compaction_keep_last` are kept verbatim. Long conversations keep working without losing the thread.
- **Concurrency:** `max_active_turns` per server, `max_active_turns_per_user` per user.

## Models

The user picks a model per conversation from the providers the server exposes. If a provider is rate-limited or failing, the turn moves to the next provider by `priority`. Usage and cost are recorded per call.

## Turns survive the browser

A turn runs on the server. Closing the tab, reloading or switching device does not stop it: reopening the conversation shows its progress and continues streaming. In a cluster, if the node running a turn goes away, another node marks the turn as interrupted and frees the conversation, which can then be resumed.

## Asking and confirming

The agent can ask the user a question with choices (`question`) when a decision is theirs. In Deploy mode, actions that change machines, such as Terraform apply or creating a container, wait for the user to confirm.

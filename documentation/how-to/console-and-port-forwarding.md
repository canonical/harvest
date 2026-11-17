# Use the agent console and port forwarding

**Goal:** get a shell on a machine or reach a service on it, through Harvest.

**Console:** in **Agents**, open an agent to get a terminal on the machine, as the agent's user (root by default). It runs over the agent's outbound connection.

**Port forwards:** on the agent's page, under **Port Forwards**, select **Add port forward** and set:

| Field | Meaning |
|---|---|
| Route | a name used in the URL |
| Port | the port on the machine |

The service is then reachable, for signed-in project members, at:

```text
https://harvest.example.com/agents/<agent-id>/<route>/
```

In Deploy chat, the agent manages forwards with `create_port_forward` and related tools.

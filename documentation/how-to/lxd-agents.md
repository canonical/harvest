# Let Harvest create agents in LXD

**Goal:** Harvest creates the machines it deploys to.

Requires the `[lxd]` section (see [Enable Code mode with LXD](enable-code-mode.md); the same LXD server is used) and `agents.public_url` reachable from the containers.

In the project, select **Add agent → Let Harvest create and manage agent**, name it and pick a size:

| Size | vCPU | Memory |
|---|---|---|
| tiny | 1 | 512 MiB |
| small | 1 | 1 GiB |
| medium | 2 | 2 GiB |
| large | 4 | 4 GiB |
| extra-large | 8 | 8 GiB |

Harvest creates the container, installs the agent and shows each step. From **Agents**, managed agents can be started, stopped, restarted and deleted; deleting removes the container.

In Deploy chat, the agent can create and delete these containers itself (`create_lxd_agent`, `delete_agent`) when you ask.

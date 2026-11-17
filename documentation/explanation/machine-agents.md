# Machine agents

A machine agent (`harvest-agent`) is a small program on a machine you manage. It lets a Deploy project run commands, Terraform and consoles there.

## Outbound only

The agent connects to Harvest; Harvest never connects to it. It holds an event stream open to `/agent/events`, sends a heartbeat to `/agent/ping`, and posts results back. Machines behind NAT or firewalls work without inbound ports, and Harvest needs no credentials for them.

If the connection drops, the agent reconnects with growing back-off. **Last seen** shows when Harvest last heard from it.

## Enrolment

The install command carries a project-specific install token. The installer exchanges it for the agent's own token, then runs the agent as a service. Rotate the install token if a command leaks; agents already enrolled are unaffected.

## What it does

| Request | Effect |
|---|---|
| execute | run a command and stream output, with a time limit |
| terraform | run plan, apply or destroy on an artifact bundle; installs Terraform when missing |
| console | an interactive terminal over a websocket tunnel |
| port forward | relay HTTP to a local port, published at `/agents/<id>/<route>/` for project members |

## Managed agents

With LXD configured, Harvest can create the machine itself: it launches a container of the chosen size, installs the agent and enrols it. Deleting the agent deletes the container.

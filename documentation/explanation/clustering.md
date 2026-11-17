# Clustering

Several Harvest servers can share one PostgreSQL database behind a load balancer, for capacity and to survive the loss of a node.

## What is shared

All durable state is in PostgreSQL: conversations, turns, code projects, deployments. Any node can serve any request. Nodes coordinate through a small membership layer:

- **who is alive**: heartbeats; a node missing for `node_timeout_ms` is considered gone;
- **who owns what**: leases for background work (ingestion, the keeper, preview expiry), so each job runs once;
- **events**: a bus that forwards turn events to the node streaming them to the browser.

## Backends

| Backend | Membership | When |
|---|---|---|
| `postgres` (default) | rows and leases in PostgreSQL, LISTEN/NOTIFY for events | simplest; the database is already shared |
| `quorium` | Raft among the nodes, with fencing tokens | stronger guarantees when nodes may be partitioned |

## Failure

Each conversation has at most one running turn, locked to a node. When a node dies mid-turn, a surviving node records the turn as interrupted and releases the lock, so the conversation can be resumed. A node that loses contact with the cluster fences itself: it fails readiness and aborts its turns. Fencing tokens only grow, so a node returning from a pause cannot overwrite newer work.

Operations: [Run Harvest as a cluster](../how-to/run-a-cluster.md). Design details: [developer/cluster.md](../developer/cluster.md).

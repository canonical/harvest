# Run Harvest as a cluster

**Goal:** several Harvest servers behind one address, surviving the loss of a node.

All nodes share one PostgreSQL database. Each node also listens on an internal address for the other nodes.

```bash
# first node
sudo harvest cluster init --internal-url http://10.0.0.11:8081
sudo harvest cluster token create          # prints a single-use join token

# every other node
sudo harvest cluster join --token <token> --internal-url http://10.0.0.12:8081

harvest cluster status                     # leader, quorum, members
harvest cluster list                       # live nodes
```

Day-to-day:

| Task | Command |
|---|---|
| Stop routing new turns to a node | `sudo harvest cluster evacuate <node>` |
| Route turns to it again | `sudo harvest cluster restore <node>` |
| Remove a node | `sudo harvest cluster remove <server-id or internal-url>` |
| Rotate the cluster secret | `harvest cluster rotate-secrets stage`, `commit --secret <new>`, `finish` on every node |
| Recover after losing quorum | `sudo harvest cluster recover` |

Restart nodes one at a time and wait for `GET /health/ready` before the next.

Membership runs on the `postgres` backend by default; `backend = "quorium"` adds a Raft-based membership with its own `data_dir`. Keys: [configuration reference](../reference/configuration.md#cluster). Design and failure handling: [developer/cluster.md](../developer/cluster.md).

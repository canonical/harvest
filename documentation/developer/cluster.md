# Clustering and high availability

Every `knowledge-server` node serves every request. PostgreSQL holds all business data
(conversations, projects, deployments, the `active_turns` locks) and must itself be highly
available. The *cluster control plane* — which nodes are alive, how events reach watchers on
other nodes — runs on one of two backends:

| | `postgres` (default) | `quorium` |
|---|---|---|
| Membership | heartbeats in `cluster_nodes` | sessions in an embedded raft group |
| Failure detection | `node_timeout_ms` after the last heartbeat (20 s by default) | a session lapses after `node_timeout_ms` (5 s in the snap and charm) |
| Event bus | `LISTEN/NOTIFY`, large payloads through `bus_payloads` | a WebSocket mesh between nodes |
| Extra state | none | `cluster.data_dir` (raft log, vote, snapshot) |
| Minimum size for fault tolerance | 2 nodes | 3 nodes |

The rest of the server does not know which backend runs: conversation locks, presence, agent
routing and ingestion jobs stay in PostgreSQL and only ask the backend for the set of live node
ids (`ClusterNode::live_node_ids`).

## Guarantees

* **One runner per conversation.** `active_turns` is taken with an atomic upsert that only
  replaces a row whose node is not alive.
* **Fencing.** A node that cannot confirm its membership for `node_timeout_ms − heartbeat_interval_ms`
  marks itself fenced: it fails readiness, refuses new turns and aborts its running turns. This
  happens before the rest of the cluster can consider it dead. A node that resumes after a pause
  notices the lapse on its next heartbeat and aborts its turns too.
* **No stale writes.** A project turn saves its messages only while its `active_turns` row still
  carries its `turn_id`; a fenced node that keeps running cannot overwrite the turn that took over.
* **Eviction is final (quorium).** Once the leader expires a session, that node id can never
  register again: the process drains and exits with an error so its supervisor restarts it under
  a new node id.
* **Bus continuity.** Each node numbers the batches it publishes. A receiver that sees a gap, or a
  reconnect that lost batches, sends `{"type":"resync"}` to its subscribers, which reload the
  project state.

## Quorium

Quorium is the `quorium` workspace crate: [openraft](https://docs.rs/openraft) with a
[redb](https://docs.rs/redb) log store, served over the internal listener under
`/internal/quorium/*` and protected by the cluster secret. Its replicated state is small and
soft — members, their sessions, join token digests — so it never carries conversation data.

* Up to five servers vote; the rest are learners. The leader promotes caught-up learners so the
  voter count stays odd (`1, 1, 3, 3, 5, 5, …`).
* Session keep-alives go to the leader and are not written to the log. Only expiry is
  committed. A new leader grants every session a full TTL before it expires anyone.
* Liveness reads used for lock take-over are linearizable (ReadIndex on the leader).

### Configuration

```toml
[cluster]
backend          = "quorium"
internal_listen  = "0.0.0.0:8081"
internal_url     = "http://10.0.0.11:8081"
shared_secret    = "…"
data_dir         = "/var/snap/harvest/common/quorium"
bootstrap        = "init"            # first server only; "join" for the others
# join_token     = "…"               # from `harvest cluster token create`
# join_addresses = ["http://10.0.0.12:8081"]   # trusted join without a token
heartbeat_interval_ms = 1000
node_timeout_ms       = 5000         # must exceed 2 × election_timeout_ms + heartbeat_interval_ms
election_timeout_ms   = 1000
# bus = "mesh"                        # default for quorium; "notify" keeps LISTEN/NOTIFY
# accepted_secrets = ["…"]           # extra secrets accepted during a rotation
```

`bootstrap` only matters while `data_dir` is empty; a restarted server rejoins with its stored
log. A join token carries the cluster secret, so `shared_secret` may be left out on a joining
server.

### Snap

```bash
# first node
sudo harvest cluster init --internal-url http://10.0.0.11:8081
sudo harvest cluster token create            # prints a single-use token
# every other node
sudo harvest cluster join --token <token> --internal-url http://10.0.0.12:8081

harvest cluster status                       # leader, quorum, voters, members
harvest cluster list                         # live nodes
sudo harvest cluster evacuate <node>         # stop routing new turns to a node
sudo harvest cluster restore <node>
sudo harvest cluster remove <server-id|internal-url>
sudo harvest cluster token revoke
```

`harvest cluster preseed --file cluster.yaml` configures a whole cluster at once: the first
system (or `cluster.bootstrap`) initializes, the others join the listed addresses with the shared
secret instead of a token.

```yaml
cluster:
  shared_secret: "…"
systems:
  - name: harvest-1
    address: http://10.0.0.11:8081
  - name: harvest-2
    address: http://10.0.0.12:8081
  - name: harvest-3
    address: http://10.0.0.13:8081
```

### Charm

Set `cluster-backend=quorium` on `harvest-server`. The first leader seeds the cluster; every
other unit joins the units it sees on the peer relation, and keeps that list once rendered so a
growing application does not restart its members. When a unit departs, the leader removes its
raft server. A rolling restart waits until restarting the unit keeps a healthy majority of voters.

`juju run harvest-server/leader cluster-rotate-secrets` rotates the shared secret in three
phases tracked in the cluster secret: every unit first accepts the new secret, then sends it, then
stops accepting the old one. Each phase advances once every unit reports (by digest) that it runs
the previous one.

## Operations

### Rolling restart

Restart one node at a time and wait for `/health/ready`. With quorium, readiness includes quorum
(`"cluster": true`), so a node is only ready once the raft group has a leader.

### Losing quorum

With two of three voters gone, the survivor fails readiness and refuses turns — on purpose: it
cannot tell a partition from a crash. Bring a voter back, or, if the others are lost for good,
recover the survivor:

1. Stop the survivor and every lost node that might come back.
2. Move `data_dir` aside on the survivor.
3. Start it with `bootstrap = "init"`. It forms a new single-voter cluster; the members
   re-register within one heartbeat. No conversation data is lost: it lives in PostgreSQL.
4. Join the other servers with fresh tokens.

### Secret rotation by hand

```bash
sudo harvest cluster rotate-secrets stage           # on every node; prints the new secret
sudo harvest cluster rotate-secrets commit --secret <new>   # on every node
sudo harvest cluster rotate-secrets finish          # on every node
```

### Metrics

`/metrics` exposes `harvest_cluster_quorum{backend=…}`, `harvest_cluster_fenced`,
`harvest_cluster_voters` next to the readiness and turn gauges, and `/health/ready` reports the
membership health of the node.

## Performance

[`ha-lab/baseline.md`](../../ha-lab/baseline.md) holds the measured load baseline and the
targets each release is checked against; rerun the load profile described there after any
change to the turn path or the cluster layer.

## Testing

* `cargo test -p quorium` runs the raft state machine, the storage and a three-server cluster in
  one process (joins, leader loss, minority partitions, eviction).
* `knowledge-server/tests/cluster_e2e.rs` starts real server processes and runs every scenario on
  both backends, including a node frozen with `SIGSTOP` mid-turn:

```bash
HARVEST_TEST_DATABASE_URL=postgres://harvest:harvest@127.0.0.1:5432/postgres \
  cargo test -p knowledge-server --test cluster_e2e -- --include-ignored --test-threads=1
```

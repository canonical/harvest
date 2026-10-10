# HA lab

A system test of an active/active Harvest cluster on one machine, using LXD:

* `harvest-pg-a` / `harvest-pg-b`: PostgreSQL 16 primary and streaming replica, TLS only,
  certificates from a throwaway CA.
* `harvest-ha-lab`: HAProxy 2.8 configured like the haproxy charm renders the
  `haproxy-route` requests of the two Harvest charms (path routing, `leastconn`,
  `/health/ready` checks, retries with redispatch, 50 s client timeout), plus the built web UI.
* Three `knowledge-server` processes on the host, one database URL listing both PostgreSQL
  hosts with `target_session_attrs=read-write`.

```bash
lxc launch ubuntu:24.04 harvest-ha-lab && lxc exec harvest-ha-lab -- apt-get install -y haproxy
lxc launch ubuntu:24.04 harvest-pg-a && lxc launch ubuntu:24.04 harvest-pg-b
ha-lab/provision.sh
SERVER_BIN=target/debug/knowledge-server node ha-lab/ha_e2e.mjs
```

Scenarios, each recorded in `~/.cache/harvest-ha-lab/report.json`:

1. HAProxy reaches all three servers for API paths and the web UI for everything else.
2. A chat turn streams identically to watchers attached to different nodes.
3. Agent commands requested through any node reach an agent connected to one node.
4. A console WebSocket survives 70 s of silence through HAProxy (client timeout 50 s).
5. `SIGKILL` of the node running a turn: the turn is aborted cluster-wide, the lock is freed,
   API traffic through HAProxy sees no failed request.
6. Rolling restart of every node with `SIGTERM`: no failed request.
7. PostgreSQL primary failover: no Harvest node restarts, traffic resumes on its own.

The failover scenario promotes the replica; run `provision.sh` on fresh containers before
running the lab again.

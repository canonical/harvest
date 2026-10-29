# Load baseline

Measured with the load profile in `knowledge-server/tests/cluster_e2e.rs`: three server
processes on one host (AMD Ryzen 9 6900HX, 16 threads), one local PostgreSQL 16, a mock
model that streams 20 chunks 20 ms apart (400 ms per answer), 400 turns, one user, a fresh
conversation per turn, the watcher on a different node from the one running the turn.
Debug builds; release builds are faster.

```bash
HARVEST_TEST_DATABASE_URL=postgres://harvest:harvest@127.0.0.1:5432/postgres \
HARVEST_LOAD_TURNS=400 HARVEST_LOAD_CONCURRENCY=8 HARVEST_LOAD_BACKEND=quorium \
  cargo test -p knowledge-server --test cluster_e2e -- --include-ignored load_profile
```

`first token` is measured from the `POST` to the first `text_delta` seen by the watcher on
another node, so it includes the cluster bus hop; `completion` ends at its `done` event.

| backend | concurrency | completed | rejected | failed | turns/s | first token p50 / p95 / p99 (ms) | completion p50 / p95 / p99 (ms) |
|---|---|---|---|---|---|---|---|
| postgres | 8 | 400 | 0 | 0 | 16.3 | 60 / 68 / 106 | 448 / 485 / 516 |
| quorium | 8 | 400 | 0 | 0 | 16.9 | 60 / 68 / 70 | 448 / 457 / 482 |
| postgres | 32 | 179 | 221 | 0 | 50.7 | 49 / 104 / 117 | 473 / 512 / 538 |
| quorium | 32 | 364 | 36 | 0 | 17.5 | 278 / 725 / 816 | 717 / 1155 / 1230 |

At concurrency 32 a single user exceeds `agent.max_active_turns_per_user` (10), so most turns
are rejected with 429 by design; how many slip through depends on how quickly turns finish,
which is why the two backends admitted different numbers of turns. Harvest's own overhead on
the time to first token is the measured value minus the mock's first chunk (immediate), i.e.
about 50–70 ms at p50 including the cross-node event hop.

## Findings from building the baseline

* The summary refresh held an advisory-lock transaction while it needed a second pool
  connection; under bursts every connection ended up waiting and requests failed after the
  15 s pool timeout. It now claims a lease column instead (`summary_lease_until`).
* Display paths (locks, presence, agent lookup) asked PostgreSQL for the live node set on
  every request; they now share a cache that lives half a heartbeat. Lock take-over and
  reaping still read fresh.
* In early runs on the PostgreSQL backend a few watchers (3 of 200, in two of six runs)
  never received any event of their conversation although the turn completed. It did not
  reproduce in later runs (2,300 turns) and never happened on the mesh bus; watch
  `harvest_mesh_resyncs_total` and the bus listener warnings if it shows up in production.

## Targets

| measure | target | where it is checked |
|---|---|---|
| Harvest overhead on the time to first token, p95 | < 150 ms | this profile, `harvest_turn_stage_milliseconds{stage="first_token"}` |
| conversation lock, p99 | < 20 ms | `harvest_turn_stage_milliseconds{stage="lock"}` |
| context load for long conversations, p95 | < 30 ms (measured 0.4 ms at 2,000 messages) | `a_long_conversation_loads_only_what_follows_its_summary`, `harvest_turn_stage_milliseconds{stage="load_context"}` |
| failure detection / take-over on quorium | < 3 s / < 5 s | `a_crashed_node_aborts_its_turn_and_releases_the_lock::quorium` (asserts < 10 s on CI hosts) |
| failed turns under load | 0 | this profile asserts it |
| rolling restart, secret rotation | no failed request | `the_cluster_secret_rotates_without_downtime` |

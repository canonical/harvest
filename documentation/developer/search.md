# Code search

Harvest answers code questions with a tiered search engine instead of a long agent loop. The
loop's work moves into a richer index and an engine whose number of rounds adapts to how hard
the question is. The answering model calls one tool, `search_code`, and answers once.

## Layout

| Crate | Owns |
|---|---|
| `harvest-search` | Indexes, retrieval, tiers, the sufficiency gate and the Postgres fallback. No LLM provider, HTTP or cluster code: it talks to the outside through `CorpusSource` (rows in) and `LanguageModel` (fast-model calls out). |
| `knowledge-server` | `search/` wires the engine from `[search]`, exposes `search_code`, records traces and metrics, keeps indexes fresh and runs shadow mode. `jobs/` runs fenced background work such as enrichment. `cluster/lease.rs` provides leases on both cluster backends. |
| `knowledge-harvester` | Parses code (Rust, Python, Go, TypeScript, JavaScript), extracts non-code units, resolves typed edges and writes them to Postgres. |
| `harvest-eval` | Builds gold sets, runs any engine over them and reports quality and latency, compares reports against gates, calibrates the gate and trains the reranker. |

Every engine implements `SearchEngine::search(&Query, Deadline) -> SearchResult`, so the legacy
loop (`LegacyLoopEngine`), the tiered engine and the SQL fallback are measured the same way.

## How a query runs

1. **Tier 1** (no model, about 100–200 ms on a 35k-symbol version) runs up to three rounds:
   BM25, dense and exact-name retrieval fused by weighted reciprocal rank, personalized PageRank
   over the typed graph with edge weights chosen by question kind, then new terms from the top
   hits. It stops when the top ten stop changing, reranks with `LinearReranker` and packs the
   best evidence into the token budget. A class replaces its own methods when both rank.
2. **The gate** runs checks cheapest first and stops at the first that decides: list and caller
   completeness against the graph, dangling references from the top evidence, a calibrated
   logistic score over retrieval signals, and optionally a small-model judge
   (`search.judge = true`). Anything still undecided escalates.
3. **Tier 2** asks the fast model once for a plan in a small typed step language
   (`search`, `subclasses_of`, `capability`, `method`, `callers`, `callees`, `reads_config`,
   `read`) and executes it against the in-memory index in milliseconds. Capability and subclass
   steps produce compact tables whose rows count as evidence for every symbol they name.
4. **Tier 3** uses the same steps one move at a time, up to `search.tier3_turns` model calls,
   with the executed history in each prompt.

The deadline (`search.deadline_ms`) is checked before every escalation. A failing model ends
the search with the evidence gathered so far (`degraded = model_unavailable`); a version whose
index is not loaded yet is answered by the SQL fallback (`degraded = index_cold`) while the
index builds in the background.

## Index

The harvester (parser version 3) stores, per version:

* symbols with their owning class (`impl_type`), so same-named methods of different classes no
  longer overwrite each other;
* typed edges: `CALLS`, `INHERITS`, `IMPLEMENTS`, `USES`, `EMBEDS`, `OVERRIDES`, `INSTANTIATES`,
  `RAISES`, `REGISTERS` and `TESTS` between symbols, resolved by an import- and class-aware
  resolver (own class and ancestors, then same file, then imported modules, then a unique name,
  and a polymorphic call resolves to the base method);
* `code_units`: module skeletons with recent commit subjects, config sections and oslo option
  declarations, documentation sections, and statement-boundary chunks of long functions;
* `unit_edges`: `READS_CONFIG` (function to option), `DOCUMENTS` (doc section to symbol),
  `REGISTERS` (entry points to classes) and `CO_CHANGES` (files changed together in git).

Versions ingested by an older parser are re-ingested automatically. Each node builds its
in-memory index (BM25, vectors, CSR graph, cache) from Postgres and caches the corpus on local
disk under `search.cache_dir`, keyed by a generation fingerprint. Restarts reload from disk;
an ingestion event on the cluster bus, or the `search.refresh_secs` poll, swaps in a new
generation. A query pins one generation for all its rounds.

`symbol_enrichments` (LLM summaries, questions and tags) and `evidence_cache` (sub-question
evidence from offline runs) are folded into the index when present. Enrichment is redone only
when a symbol's source hash changes and is copied across versions by hash. A cache entry is
dropped as soon as any symbol it cites changes.

## Configuration

```toml
[search]
engine = "shadow"            # loop (default) | shadow | tiered
deadline_ms = 7000           # retrieval budget; the answer call comes on top
token_budget = 8000
cache_dir = "/var/snap/harvest/common/search-cache"
tier3_turns = 6
judge = false                # ask the fast model when retrieval signals are undecided
enrich = false               # background LLM enrichment through the work queue
reranker = "/etc/harvest/reranker.json"   # from harvest-eval train-reranker
gate = "/etc/harvest/gate.json"           # from harvest-eval calibrate-gate
warm = ["cinder@29.0.0"]     # loaded at start, besides recently searched versions
refresh_secs = 30

[search.fast_model]
provider_id = "anthropic"
model = "claude-haiku-5-5"
```

* `loop` keeps today's agent loop and tools.
* `shadow` keeps the loop for users, runs the tiered engine on the same question after each turn
  and records how much of the loop's cited evidence it found (`shadow_recall`).
* `tiered` gives the agent `search_code`, `read_sources`, `compare_symbol_across_versions`,
  `list_repositories` and `run_sql`, and adds a short prompt section telling it to search once.

Roll out as `shadow`, compare `harvest_search_shadow_recall_percent`, then switch to `tiered`.

## High availability

* Indexes are derived data that every node builds for itself; losing a node loses only its warm
  cache.
* Exclusive work runs under leases (`cluster/lease.rs`): Quorium leases are bound to the holder's
  session and carry monotonic fencing tokens; the Postgres backend keeps the same contract with a
  lease table reaped by liveness. `run_exclusive` stops its job when the lease is lost.
* Background work goes through `work_queue` (`FOR UPDATE SKIP LOCKED`). Items held by a dead node
  are reaped, each item runs under its own lease, and its writes commit through `Fence::commit`,
  which refuses a token older than the newest one written for that key. Nodes stop claiming
  work while they are busy with turns, draining or fenced.
* Quorium only proposes lease commands once every member advertises the `leases` feature, so a
  rolling upgrade never sends an old node a command it cannot apply.

## Observability

`/metrics` adds `harvest_search_milliseconds{engine,tier}`,
`harvest_search_stage_milliseconds{stage}`, `harvest_search_tier_total`,
`harvest_search_degraded_total{reason}`, `harvest_search_llm_calls_total{tier}`,
`harvest_gate_decision_total{tier,check,verdict}`, `harvest_search_shadow_recall_percent`,
`harvest_search_index_load_milliseconds`, `harvest_search_index_staleness_milliseconds`,
`harvest_search_index_loaded{repo,version}`, `harvest_lease_acquire_total`,
`harvest_fenced_write_rejected_total`, `harvest_workqueue_depth{kind}`,
`harvest_workqueue_reaped_total` and `harvest_workqueue_items_total{kind,outcome}`.

Every search is also stored in `search_traces`: query, scope, tier reached, degradation,
stage timings, gate decisions, signals, evidence keys and, in shadow mode, the shadow recall.

## Measuring quality and latency

```bash
# gold sets: synthetic from the graph, from conversation logs, or distilled from offline runs
harvest-eval synth   --database-url $DB --repo cinder --version 29.0.0 --out gold.jsonl
harvest-eval extract --database-url $DB --out logs.jsonl
harvest-eval distill --config server.toml --gold questions.jsonl --engine legacy --cache --out offline.jsonl

# runs: every report holds per-query records and a summary
harvest-eval run --database-url $DB --gold gold.jsonl --split dev --engine tier1  --out tier1.json
harvest-eval run --config server.toml --gold gold.jsonl --split dev --engine tiered --label-gate --out tiered.json
harvest-eval run --config server.toml --gold gold.jsonl --split dev --engine legacy --answers --out loop.json
harvest-eval run --config server.toml --gold gold.jsonl --split dev --engine tiered --judge --references loop.json --out judged.json

# gates: exits non-zero when one fails
harvest-eval compare loop.json tiered.json

# calibration and training (train split only)
harvest-eval train-reranker --database-url $DB --gold gold.jsonl --split train --out reranker.json
harvest-eval run --database-url $DB --reranker reranker.json --gold gold.jsonl --split train --engine tier1 --out train.json
harvest-eval calibrate-gate --report train.json --out gate.json
```

Splits are fixed by hashing each query id (30% held out, 30% dev, 40% train); tune on `dev`
and `train`, report on `heldout`.

| Metric | Meaning | Gate |
|---|---|---|
| Essential recall | Share of essential gold evidence in the packed context. A symbol matches by qualified name (bare or qualified), a span by at least half its lines, and table rows count for the symbols they cover. | ≥ 95% of the loop's when the base report is `legacy`; otherwise the 95% bootstrap interval of the paired change must stay above −1 point |
| Pool recall | The same over the top 200 candidates, to tell retrieval losses from packing losses | diagnostic |
| nDCG@10, precision | Ranking quality and the share of packed text that is gold | diagnostic |
| Answer margin | Blind pairwise judge in both orders, win = +1, loss = −1 | lower bound of the 95% interval ≥ −0.03 |
| Latency | End-to-end retrieval p50/p90/p99, per tier and per stage, and the share within budget | p99 ≤ 10 s, tier 1 p99 ≤ 500 ms |
| Model calls | Most calls per tier | tier 1: 0, tier 2: 2, tier 3: 8 |
| Missed insufficiency | Tier 1 evidence below 90% recall that the gate still accepted | ≤ 5% |

### Baseline on Cinder 29.0.0

287 synthetic questions (lookup, docstring description, subclass lists, callers and call flow)
generated from the version's graph and kept fixed; numbers are on the 86 held-out questions,
retrieval only, release build, one local Postgres. Reports are in `eval/reports/`.

| | Old index, default ranking | New index, trained reranker, calibrated gate |
|---|---|---|
| Essential recall | 0.791 | 0.847 |
| nDCG@10 | 0.538 | 0.856 |
| Precision | 0.094 | 0.134 |
| Tier 1 latency p50 / p99 | 72 / 122 ms | 122 / 173 ms |
| Missed insufficiency | 13.0% | 0.0% |
| Unneeded escalation | 6.3% | 88.6% |

The SQL fallback took 5.3 s per question on a 50-question sample, which is why it only serves
cold indexes. The calibrated gate escalates most questions because retrieval signals alone do
not separate confident hits from confident misses on this set; the small-model judge is the
layer meant for that, and needs a model to measure. Answer quality and the comparison against
the legacy loop need an LLM provider and were not measured here.

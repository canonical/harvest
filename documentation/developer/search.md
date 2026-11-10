# Code search

Harvest answers code questions with a tiered search engine instead of a long agent loop. The
loop's work moves into a richer index and an engine whose number of rounds adapts to how hard
the question is. The answering model calls one tool, `search_code`, and answers once.

## Layout

| Crate | Owns |
|---|---|
| `harvest-search` | Indexes, retrieval, tiers, the sufficiency gate and the Postgres fallback. No LLM provider, HTTP or cluster code: it talks to the outside through `CorpusSource` (rows in) and `LanguageModel` (fast-model calls out). |
| `knowledge-server` | `search/` wires the engine from `[search]`, exposes `search_code`, records traces and metrics, keeps indexes fresh and runs shadow mode. `jobs/` runs fenced background work such as enrichment. `cluster/lease.rs` provides leases on both cluster backends. |
| `knowledge-harvester` | Parses code in 44 languages, classifies each file as source, test, vendored or generated, extracts non-code units, resolves typed edges and writes them to Postgres. |
| `harvest-eval` | Builds gold sets, runs any engine over them and reports quality and latency, compares reports against gates, calibrates the gate and trains the reranker. |

Every engine implements `SearchEngine::search(&Query, Deadline) -> SearchResult`, so the legacy
loop (`LegacyLoopEngine`), the tiered engine and the SQL fallback are measured the same way.

## How a query runs

1. **Tier 1** (no model, about 100–200 ms on a 35k-symbol version) runs up to three rounds:
   BM25, dense and exact-name retrieval fused by weighted reciprocal rank, personalized PageRank
   over the typed graph with edge weights chosen by question kind, then new terms from the top
   hits. Symbols the question names, in any language's syntax (`Fence::commit`, `a.b.C`,
   `obj->run`, `Class#method`), are matched by name parts and pinned. It stops when the top ten
   stop changing, then reranks and packs at most twelve pieces into the token budget: whole
   functions with the class they belong to and the one-line signatures of what they call, and
   large classes as a skeleton of member signatures. Pieces from the same file sit together.
   List and caller questions get up to 32 shorter pieces instead. A list question about a family
   of classes ("which drivers…") gets the complete family: the base class with the most members
   related to the question, found among the top hits and among classes named after a word of
   the question, is expanded into a table of every non-test subclass. When the question also asks
   about a property ("…support active-active"), a class-level attribute of that base class whose
   name matches the question is resolved for every member, including inherited values.
2. **The gate** runs checks cheapest first and stops at the first that decides: symbols named in
   the question that are missing from the evidence, list and caller completeness against the
   graph, dangling references from the top evidence, a calibrated logistic score over retrieval
   signals, and optionally a small-model judge (`search.judge = true`). Anything still undecided
   escalates.
3. **Tier 2** asks the fast model once for a plan in a small typed step language
   (`search`, `subclasses_of`, `capability`, `method`, `callers`, `callees`, `reads_config`,
   `read`) and executes it against the in-memory index in milliseconds. Capability and subclass
   steps produce compact tables whose rows count as evidence for every symbol they name.
4. **Tier 3** uses the same steps one move at a time, up to `search.tier3_turns` model calls,
   with the executed history in each prompt.

The deadline (`search.deadline_ms`) is checked before every escalation and, in Tier 3, before
every model call, using the duration of the previous call. Running out of time keeps the
evidence gathered so far (`degraded = deadline_truncated`). A failing model ends
the search with the evidence gathered so far (`degraded = model_unavailable`); a version whose
index is not loaded yet is answered by the SQL fallback (`degraded = index_cold`) while the
index builds in the background.

## Index

The harvester (parser version 4) stores, per version:

* symbols with their owning class (`impl_type`) and whether they are test code, so same-named
  methods of different classes no longer overwrite each other;
* the origin of every file: `source`, `test`, `vendored` or `generated`, decided from
  `.gitattributes` (`linguist-vendored`, `linguist-generated` and their negations), generated
  markers (`@generated`, `DO NOT EDIT`, minified text), GitHub Linguist's vendored path
  conventions, git history (files only ever changed by dependency-sync commits) and test path
  conventions;
* typed edges: `CALLS`, `INHERITS`, `IMPLEMENTS`, `USES`, `EMBEDS`, `OVERRIDES`, `INSTANTIATES`,
  `RAISES`, `REGISTERS` and `TESTS` between symbols, resolved by an import- and class-aware
  resolver (own class and ancestors, then same file, then imported modules, then a unique name,
  and a polymorphic call resolves to the base method);
* `code_units`: module skeletons with recent commit subjects, config sections and oslo option
  declarations, documentation sections, chunks of long functions split at the statements of
  their syntax tree, and line windows of source files no parser understands;
* `unit_edges`: `READS_CONFIG` (function to option), `DOCUMENTS` (doc section to symbol),
  `REGISTERS` (entry points to classes) and `CO_CHANGES` (files changed together in git).

### Languages

Rust, Python, Go, TypeScript and JavaScript have dedicated parsers. Every other language goes
through one generic parser (`parser/generic.rs`): it uses the grammar's `tags.scm` query where
the crate ships one, and a structural pass over the syntax tree that recognises definitions,
calls, base types, imports, annotations and leading doc comments from node and field names. It
covers C, C++, Java, C#, Ruby, PHP, Kotlin, Scala, Swift, Lua, Bash, Elixir, Haskell, OCaml,
Julia, Elm, Dart, R, Zig, Objective-C, Erlang, Solidity, Fortran, Groovy, Perl, D, Gleam, F#,
Racket, Protocol Buffers, Verilog, Pascal, Nix, PowerShell, CUDA, GLSL, Scheme, Ada and MATLAB
(`.m` files are told apart from Objective-C by content). Adding a language means adding a
grammar crate and one line to `generic::specs()`. Test code is recognised in the parser from
names, attributes or decorators (`#[test]`, `@Test`, `[Fact]`, `pytest` markers) and enclosing
test scopes such as `#[cfg(test)] mod` or `*Tests` classes. A file whose language has no
grammar, or whose parser finds no definitions, is indexed as line windows.

Search never looks at file paths or syntax: it reads the language-neutral fields the harvester
writes (kind, owner, name parts, test flag, origin).

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
max_followups = 3            # code reads allowed after the first search, per question
prefetch = true              # run the first search before the model's first call
calibrate = true             # calibrate search per repository in the background

[search.fast_model]
provider_id = "anthropic"
model = "claude-haiku-5-5"
```

* `loop` keeps today's agent loop and tools.
* `shadow` keeps the loop for users, runs the tiered engine on the same question after each turn
  and records how much of the loop's cited evidence it found (`shadow_recall`).
* `tiered` gives the agent `search_code`, `read_sources`, `compare_symbol_across_versions` and
  `list_repositories` with a prompt of its own. When the question names an ingested repository,
  or only one is ingested, the server runs the first search before the model's first call and
  hands the evidence over with the question. After that the agent may make
  `search.max_followups` code reads (`search_code` with `focus` on named symbols or files,
  `read_sources`, version comparisons) before it must answer; further reads are refused with a
  message telling it to answer from what it has.

Roll out as `shadow`, compare `harvest_search_shadow_recall_percent`, then switch to `tiered`.

## Calibration

The reranker weights and the gate thresholds come from, in order: a calibration stored for the
version in `search_calibrations`, the files named by `search.reranker` and `search.gate`, and the
defaults built into `harvest-search/defaults/calibration.json`, which are trained across several
open-source repositories in different languages.

With `search.calibrate = true`, a fenced background job (`jobs/kinds/calibrate.rs`) calibrates
each ingested version once its content changes. It generates questions from the index itself —
symbol lookups, docstring descriptions, subclass lists, callers and call flows — trains the
reranker on 70% of them, keeps the trained weights only if they do at least as well as the
current ones on the other 30%, calibrates the gate, and stores the result. The new calibration
changes the index generation, so every node picks it up on its next refresh. Calibration runs on
a blocking thread so it cannot delay cluster heartbeats, stops as soon as its lease is lost, uses
fewer questions on versions above 20,000 units and gives up after three minutes. Index builds and
the on-disk cache are also built off the async threads.

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

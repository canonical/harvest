# Improve search with enrichment and embeddings

**Goal:** better answers on large or unfamiliar codebases.

Both are optional and cost model calls.

## Tiered search with enrichment

```toml
[search]
engine = "tiered"            # "loop" (default), "tiered" or "shadow"
enrich = true                # summarise symbols in the background
[search.fast_model]
provider_id = "openrouter"   # a provider id from [[llm]]
model       = "anthropic/claude-haiku-5.5"
```

`tiered` replaces the agent's many small graph lookups with one `search_code` tool that retrieves in rounds (names, then structure, then a fast model) within a deadline (`deadline_ms`, default 7 s) and a token budget (`token_budget`, default 8000). `enrich` has the fast model write a short summary, likely questions and tags for each symbol; summaries are reused when the code is unchanged. `shadow` keeps the loop engine for answers and runs tiered search alongside it, to compare.

## Semantic search

```toml
[semantic]
enabled           = true
model             = "text-embedding-004"
backfill_on_start = true
```

Needs the `vector` extension in PostgreSQL and a configured `gemini` provider with a server key: embeddings come from Gemini. Search then blends lexical (60 %) and semantic (40 %) scores; tune with `lexical_weight` and `semantic_weight`.

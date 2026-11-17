# Costs

Harvest's main running cost is model usage. Everything else is your own infrastructure: PostgreSQL, the server, and LXD capacity for Code and Deploy.

## Where model calls come from

| Source | Rough share | Lever |
|---|---|---|
| Agent turns | most | model choice; `max_iterations`; tiered search |
| Design and provisioning generation | large per deployment, rare | model choice |
| Enrichment (`search.enrich`) | once per symbol, reused across versions | off by default; use a fast model |
| Embeddings (`semantic`) | once per symbol | off by default |
| Compaction | small, in long conversations | `compaction_threshold_chars` |

Re-indexing reuses summaries and embeddings of unchanged code, so keeping branches fresh costs little.

**Prompt caching** matters most. Each model call re-sends the whole conversation, so input is over 95% of the tokens of a coding turn. With caching, the repeated part is read from the provider's cache at a fraction of the price: in measured coding turns, 82–90% of input came from the cache. It is on by default for Claude models.

In Code mode, each turn's briefing also lists the files the code graph cannot see, the repository's test and check commands, and the files the previous turn worked on, so the agent spends fewer calls finding them again.

## Accounting

Every call is recorded with tokens (input, output, cache read, cache write, reasoning) and cost from the provider's pricing. Totals are available per project, conversation, deployment and API call ([how-to](../how-to/track-spending.md)).

## Who pays

With server keys, the organisation pays for everyone. With [user-provided keys](../how-to/user-provided-keys.md), each user's calls go to their own account.

## LXD capacity

Each session is a container; each project keeps `pool_size` warm containers and an image; each shared preview viewer may get a copy. Lower `pool_size` and `max_preview_copies` on small hosts.

# How Harvest understands code

## Ingestion

For each repository version (a branch or tag), the harvester:

1. clones or fetches the repository with the adding user's credentials;
2. parses each source file with a tree-sitter grammar ([44 languages](../reference/languages.md));
3. records **symbols** (functions, methods, classes, types, with their source range, docstring and decorators), **imports**, **calls** and **inheritance** as edges between symbols;
4. indexes everything else as text.

Versions are stored side by side. A file unchanged between versions is shared, so many versions of a large repository stay affordable, and comparisons between versions are exact.

## Search

Questions rarely name the exact symbol. Harvest combines:

- **name and text search** with trigram indexes (`pg_trgm`), tolerant to partial names and typos;
- **graph walks**: from a symbol to its callers, callees, imports, subclasses;
- optionally **semantic search** (embeddings in pgvector), blended with lexical scores;
- optionally **enrichment**: a fast model writes a summary, likely questions and tags per symbol, so natural-language questions match code that never uses their words.

## Two search engines

| Engine | How the agent searches |
|---|---|
| `loop` (default) | the agent calls small tools (`search_symbols`, `find_callers`, `read_sources`…) and decides each next step |
| `tiered` | one `search_code` tool runs rounds (names, then structure, then a fast model) within a time and token budget, and returns ranked evidence |

`loop` is flexible and transparent; `tiered` is faster and cheaper on large codebases. `shadow` runs both to compare.

## Grounding

Every result carries repository, version, file and line range. The agent is instructed to cite what it read, and the UI turns citations into links to the exact lines. An answer without citations is a signal to check it.

# knowledge-harvester Design

## Configuration

The harvester reads a `harvester.toml` file.

```toml
[neo4j]
uri      = "bolt://localhost:7687"
user     = "neo4j"
password = "secret"

[storage]
clone_root = "/var/harvest/repos"   # local cache for git checkouts

[[repositories]]
url  = "https://github.com/org/repo-a"
name = "repo-a"                     # used as the graph identifier

[[repositories]]
url  = "https://github.com/org/repo-b"
name = "repo-b"
```

### Limiting which versions are ingested

By default the harvester ingests every git **tag** in a repository. To restrict ingestion to a specific set of git refs, add a `refs` list to the `[[repositories]]` block:

```toml
[[repositories]]
name = "repo-a"
url  = "https://github.com/org/repo-a"
refs = ["v2.0.0", "v2.1.0", "main"]
```

Each entry in `refs` can be a **tag name**, a **branch name**, or a full **commit SHA**. When `refs` is present, only the listed refs are harvested; all other tags and branches are ignored. Omit `refs` entirely to revert to the default behaviour of harvesting all tags.

> **Branches and re-ingestion**: a branch ref (e.g. `main`) is stored in the graph under that name. If the branch has moved forward since the last run, use the `reingest` command to reset all versions and force re-processing on the next `run`.

### Documentation generation config

To use the `document` subcommand, add `[llm]` and `[documentation]` sections:

```toml
[llm]
provider     = "anthropic"           # "anthropic" | "openai-compatible"
model        = "claude-sonnet-4-6"
api_key      = "sk-ant-..."
timeout_secs = 300                   # per-request timeout (default: 300)
max_retries  = 3                     # retry attempts on transient errors

# — or — OpenAI-compatible (Groq, Ollama, etc.)
# [llm]
# provider = "openai-compatible"
# base_url = "https://api.groq.com/openai/v1"
# api_key  = "gsk_..."
# model    = "llama-3.3-70b-versatile"

[documentation]
docs_dir = "/var/harvest/docs"       # output directory for generated pages
```

---

## Commands

```bash
# Single harvest pass — ingest all repos once, skip already-ingested versions
cargo run -- --config harvester.toml run

# Force re-ingest all files, even already-processed versions
cargo run -- --config harvester.toml run --force

# Watch mode — poll every N seconds for new refs
cargo run -- --config harvester.toml watch --interval-secs 300

# Show ingestion status for all repos/versions
cargo run -- --config harvester.toml status

# Mark all ingested versions as pending so the next run re-processes them
cargo run -- --config harvester.toml reingest

# Generate Diataxis documentation for a specific repo:version
cargo run -- --config harvester.toml document my-repo:v1.2.0
```

---

## Ingestion Pipeline

```
for each repository:
  1. git clone / git fetch ──► local checkout
  2. list git refs (all tags, or the explicit refs list)
  3. for each ref:
       a. already in Neo4j with ingested: true?  ──► skip (unless --force)
       b. git checkout <ref>
       c. walk source files
       d. parse each file with tree-sitter
       e. build in-memory graph for this version
       f. write graph to Neo4j (batched UNWIND queries)
       g. set Version.ingested = true
```

Each `(repo, version)` ingestion is an atomic unit — a partial write leaves `ingested: false` on the `Version` node so the harvester re-processes it on the next run.

---

## Documentation Pipeline

The `document` subcommand runs after ingestion and requires `[llm]` and `[documentation]` configuration.

```
document <repo:version>:
  1. Query Neo4j for the file and symbol structure of the given version
  2. Phase 1 — identify features:
       LLM analyses file/symbol list → JSON array of 3–8 named features
  3. Phase 2+3 — describe features and intent:
       For each feature, LLM reads relevant source → detailed description + intent
  4. Phase 4 — generate Diataxis sections:
       For each of: tutorials, how-to-guides, explanations, reference
         a. LLM plans 1–4 page titles + filenames for the section
         b. LLM writes full markdown content for each page
         c. Pages written to docs_dir/<repo>/<version>/<section>/
  5. Write docs_dir/<repo>/<version>/index.json
       (repo, version, generated_at, sections → [{filename, title}])
```

The generated directory is served verbatim by the knowledge-server when `[documentation] docs_dir` is configured there too (see [server.md](server.md)).

---

## Git Integration

- Clone uses `git2-rs` (libgit2 bindings). No `git` subprocess dependency.
- Shallow clones are **not** used — full history is required to check out arbitrary refs.
- Tags are listed with `git_repository.tag_names(None)`. Branches and SHAs are resolved with `git_repository.revparse_single`.
- The resolved commit SHA and tagger/committer timestamp are stored on the `Version` node.

---

## Source File Discovery

After checkout, the harvester walks the working tree and collects files by extension. Files inside `.gitignore`-d paths are skipped (libgit2 status flags). Binary files are skipped.

Supported languages:

| Language   | Extensions              | tree-sitter grammar crate         |
|------------|-------------------------|-----------------------------------|
| Rust       | `.rs`                   | `tree-sitter-rust`                |
| Python     | `.py`                   | `tree-sitter-python`              |
| TypeScript | `.ts`, `.tsx`           | `tree-sitter-typescript`          |
| JavaScript | `.js`, `.jsx`           | `tree-sitter-javascript`          |
| Go         | `.go`                   | `tree-sitter-go`                  |
| C          | `.c`, `.h`              | `tree-sitter-c`                   |
| C++        | `.cpp`, `.cc`, `.h`     | `tree-sitter-cpp`                 |

Files in unrecognised languages are stored as plain `File` nodes (path only, no symbol extraction).

---

## tree-sitter Parsing

For each source file, the harvester queries the tree-sitter syntax tree with language-specific **query patterns** to extract:

- **Function / method definitions** — name, kind, signature, start line, end line, full source text. The `impl_type` field is set when a function belongs to an `impl` block, enabling class-containment inference in the graph API.
- **Class / struct / trait / interface definitions** — name, kind, start line, end line, full source text. The `bases`, `traits`, `embeds`, and `uses` fields carry the names of related types discovered by the parser, which are resolved to relationship edges at write time.
- **Import / use declarations** — imported path or module name.
- **Function calls** — callee name and call-site line (best-effort; resolved by name within the same version).

---

## Knowledge Graph Schema

### Node Labels

| Label        | Properties                                                                                          |
|--------------|-----------------------------------------------------------------------------------------------------|
| `Repository` | `name`, `url`                                                                                       |
| `Version`    | `repo`, `tag`, `commit_sha`, `timestamp`, `ingested`                                               |
| `File`       | `repo`, `version`, `path`, `language`                                                              |
| `Function`   | `repo`, `version`, `file`, `name`, `kind`, `signature`, `start_line`, `end_line`, `source`, `impl_type` |
| `Class`      | `repo`, `version`, `file`, `name`, `kind`, `start_line`, `end_line`, `source`                     |
| `Import`     | `repo`, `version`, `file`, `target`, `line`                                                        |

`kind` on `Function` distinguishes `function`, `method`, `constructor`, etc. `kind` on `Class` distinguishes `class`, `struct`, `trait`, `interface`, `enum`, `module`, `impl`, `type`. The `(repo, version, file, name)` tuple is a unique key for `Function` and `Class` nodes.

### Relationship Types

| Relationship  | From       | To         | Properties         | Notes                                         |
|---------------|------------|------------|--------------------|-----------------------------------------------|
| `HAS_VERSION` | Repository | Version    | —                  |                                               |
| `HAS_FILE`    | Version    | File       | —                  |                                               |
| `DEFINES`     | File       | Function   | —                  |                                               |
| `DEFINES`     | File       | Class      | —                  |                                               |
| `CALLS`       | Function   | Function   | `line` (call site) | Best-effort; resolved by name within version  |
| `IMPORTS`     | File       | File       | —                  |                                               |
| `INHERITS`    | Class      | Class      | —                  | Superclass / parent class                     |
| `IMPLEMENTS`  | Class      | Class      | —                  | Trait / interface implementation              |
| `EMBEDS`      | Class      | Class      | —                  | Struct field of another struct type           |
| `USES`        | Class      | Class      | —                  | Dependency reference (e.g. type parameter)    |

`CALLS` edges are best-effort: they link by callee name within the same version. `INHERITS`, `IMPLEMENTS`, `EMBEDS`, and `USES` edges are derived from the `bases`, `traits`, `embeds`, and `uses` lists on `ClassNode` and resolved to existing `Class` nodes at write time. Unresolved targets are silently dropped.

### Neo4j Indexes

```cypher
CREATE INDEX repo_name   FOR (r:Repository) ON (r.name);
CREATE INDEX version_tag FOR (v:Version)    ON (v.repo, v.tag);
CREATE INDEX file_path   FOR (f:File)       ON (f.repo, f.version, f.path);
CREATE INDEX fn_name     FOR (f:Function)   ON (f.repo, f.version, f.name);
CREATE INDEX cls_name    FOR (c:Class)      ON (c.repo, c.version, c.name);
```

Full-text indexes are created on `Function.name`, `Class.name`, and `File.path` to support substring search from the agent tools.

---

## Write Strategy

Graph writes use **batched `UNWIND` Cypher** to avoid N+1 round-trips. Each file's extracted symbols are sent as a single parameterised query. Nodes are created with `MERGE` (idempotent), so re-running the harvester on an already-ingested version is a no-op.

A version is marked `ingested: true` on its `Version` node only after all its files complete. If the process is interrupted mid-version, the incomplete version node retains `ingested: false` and is re-processed on the next run.

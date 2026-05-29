# knowledge-harvester Design

## Configuration

The harvester reads a `harvester.toml` file. Each repository entry specifies the remote URL and optional local clone path override.

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

Each entry in `refs` can be a **tag name**, a **branch name**, or a full **commit SHA** — anything that `git rev-parse` can resolve. When `refs` is present, only the listed refs are harvested; all other tags and branches are ignored. Omit `refs` entirely to revert to the default behaviour of harvesting all tags.

> **Branches and re-ingestion**: a branch ref (e.g. `main`) is stored in the graph under that name. If the branch has moved forward since the last run, the harvester will not automatically re-ingest it because the `(repo, ref-name)` pair is already marked `ingested: true`. To force re-ingestion of an updated branch, delete the corresponding `Version` node in Neo4j and run the harvester again.

## Pipeline

```
for each repository:
  1. git clone / git fetch ──► local checkout
  2. git tag --list          ──► version list
  3. for each tag:
       a. already in Neo4j?  ──► skip
       b. git checkout <tag>
       c. walk source files
       d. parse each file with tree-sitter
       e. build in-memory graph for this version
       f. write graph to Neo4j (batched)
```

Steps 3e and 3f for different versions of the same repo can be parallelized. Each (repo, version) ingestion is an atomic unit — a partial write is rolled back so the harvester is safe to interrupt and re-run.

## Git Integration

- Clone uses `git2-rs` (libgit2 bindings). No `git` subprocess dependency.
- Shallow clones are **not** used — full history is required to check out arbitrary tags.
- Tags are listed with `git_repository.tag_names(None)` and stored as the canonical version identifier alongside the resolved commit SHA and tagger timestamp.

## Source File Discovery

After checkout, the harvester walks the working tree and collects files by extension. Files inside `.gitignore`-d paths are skipped (libgit2 status flags). Binary files are skipped.

Supported languages in the first iteration:

| Language   | Extensions         | tree-sitter grammar crate         |
|------------|--------------------|-----------------------------------|
| Rust       | `.rs`              | `tree-sitter-rust`                |
| Python     | `.py`              | `tree-sitter-python`              |
| TypeScript | `.ts`, `.tsx`      | `tree-sitter-typescript`          |
| JavaScript | `.js`, `.jsx`      | `tree-sitter-javascript`          |
| Go         | `.go`              | `tree-sitter-go`                  |
| C          | `.c`, `.h`         | `tree-sitter-c`                   |
| C++        | `.cpp`, `.cc`, `.h`| `tree-sitter-cpp`                 |

Files in unrecognised languages are stored as plain `File` nodes (path only, no symbol extraction).

## tree-sitter Parsing

For each source file, the harvester queries the tree-sitter syntax tree with language-specific **query patterns** to extract:

- **Function / method definitions** — name, parameters (as text), return type (as text), start line, end line.
- **Class / struct / trait / interface definitions** — name, start line, end line.
- **Import / use declarations** — imported path or module name.
- **Function calls** — callee name and call site line (best-effort; resolving to a canonical definition is done at graph-link time).

The raw source text for each extracted symbol is stored on the node so the server can return it verbatim.

## Knowledge Graph Schema

### Node Labels

| Label        | Properties                                                                 |
|--------------|----------------------------------------------------------------------------|
| `Repository` | `name`, `url`                                                              |
| `Version`    | `repo`, `tag`, `commit_sha`, `timestamp`                                   |
| `File`       | `repo`, `version`, `path`, `language`                                      |
| `Function`   | `repo`, `version`, `file`, `name`, `signature`, `start_line`, `end_line`, `source` |
| `Class`      | `repo`, `version`, `file`, `name`, `start_line`, `end_line`, `source`     |
| `Import`     | `repo`, `version`, `file`, `target`, `line`                               |

The `(repo, version, file, name)` tuple is a unique key for `Function` and `Class` nodes.

### Relationship Types

| Relationship   | From         | To           | Properties       |
|----------------|--------------|--------------|------------------|
| `HAS_VERSION`  | `Repository` | `Version`    | —                |
| `HAS_FILE`     | `Version`    | `File`       | —                |
| `DEFINES`      | `File`       | `Function`   | —                |
| `DEFINES`      | `File`       | `Class`      | —                |
| `CALLS`        | `Function`   | `Function`   | `line` (call site) |
| `IMPORTS`      | `File`       | `File`       | —                |
| `MEMBER_OF`    | `Function`   | `Class`      | —                |

`CALLS` edges are best-effort: they link by callee name within the same version. Cross-file resolution follows `IMPORTS` edges. Unresolved call targets are stored with a `?unresolved` suffix on the target name so agents can flag them.

### Neo4j Indexes

```cypher
CREATE INDEX repo_name   FOR (r:Repository) ON (r.name);
CREATE INDEX version_tag FOR (v:Version)    ON (v.repo, v.tag);
CREATE INDEX file_path   FOR (f:File)       ON (f.repo, f.version, f.path);
CREATE INDEX fn_name     FOR (f:Function)   ON (f.repo, f.version, f.name);
CREATE INDEX cls_name    FOR (c:Class)      ON (c.repo, c.version, c.name);
```

Full-text indexes are created on `Function.name`, `Class.name`, and `File.path` to support substring search from the agent tools.

## Write Strategy

Graph writes use **batched `UNWIND` Cypher** to avoid N+1 round-trips. Each file's extracted symbols are sent as a single parameterised query. Nodes are created with `MERGE` (idempotent), so re-running the harvester on an already-ingested version is a no-op.

A version is marked `ingested: true` on its `Version` node only after all its files complete. If the process is interrupted mid-version, the incomplete version node has `ingested: false` and is re-processed on the next run.

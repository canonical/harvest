# knowledge-server Design

## HTTP API

The server exposes a minimal REST API.

### `POST /query`

Submit a natural-language question about one or more codebases.

**Request body:**

```json
{
  "query": "How does authentication work in repo-a v2.1.0?",
  "repositories": ["repo-a"],       // optional filter; omit to search all
  "versions":     ["v2.1.0"],       // optional filter; omit to search all versions
  "stream":        false            // if true, stream answer tokens as SSE
}
```

**Response (non-streaming):**

```json
{
  "answer": "Authentication in repo-a v2.1.0 uses ...",
  "sources": [
    { "repo": "repo-a", "version": "v2.1.0", "file": "src/auth/middleware.rs", "line": 42 },
    { "repo": "repo-a", "version": "v2.1.0", "file": "src/auth/token.rs",      "line": 17 }
  ],
  "tool_calls_made": 6
}
```

**Streaming (SSE):** events are `data: {"delta": "..."}` chunks followed by a final `data: {"sources": [...]}` event.

### `GET /repositories`

Returns the list of all ingested repositories and their available versions.

```json
[
  { "name": "repo-a", "versions": ["v1.0.0", "v1.1.0", "v2.1.0"] },
  { "name": "repo-b", "versions": ["v0.9.0", "v1.0.0"] }
]
```

### `GET /health`

Returns `200 OK` with `{"status": "ok"}`.

---

## LLM Provider Abstraction

The server defines a `LlmProvider` trait:

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        messages: &[Message],
        tools:    &[ToolDefinition],
    ) -> Result<LlmResponse>;
}
```

Two implementations ship out of the box:

### `AnthropicProvider`

Uses the Anthropic Messages API with native tool use. The model is configurable.

```toml
[llm]
provider = "anthropic"
model    = "claude-sonnet-4-6"      # or claude-opus-4-7, claude-haiku-4-5-20251001
api_key  = "${ANTHROPIC_API_KEY}"   # env var reference
```

### `OpenAiCompatProvider`

Uses the OpenAI Chat Completions API with function calling. Works with any compatible endpoint: Groq, Ollama, local vLLM, etc.

```toml
[llm]
provider = "openai-compat"
base_url = "https://api.groq.com/openai/v1"
api_key  = "${GROQ_API_KEY}"
model    = "llama-3.3-70b-versatile"
```

For local Ollama with no auth:

```toml
[llm]
provider = "openai-compat"
base_url = "http://localhost:11434/v1"
api_key  = "ollama"
model    = "qwen2.5-coder:32b"
```

---

## Agentic Workflow

The query handler runs the following loop:

```
1. Build system prompt (role, graph schema summary, citation format rules)
2. Append user query as first user message
3. Loop:
   a. Call LLM with current message history + tool definitions
   b. If response is a plain message → done, return answer
   c. If response contains tool calls:
        - execute each tool call against Neo4j
        - append tool results to message history
        - continue loop
4. Parse [repo:version:file:line] citations from final answer
5. Return structured response
```

A configurable `max_iterations` cap (default: 20) prevents runaway loops. If the cap is hit, the server returns whatever partial answer the LLM has accumulated.

### System Prompt

The system prompt tells the LLM:

- It is a code analysis assistant with access to a Neo4j knowledge graph.
- The graph schema (node labels, relationships, available properties).
- It must cite every factual claim about code in the format `[repo:version:file:line]`.
- It should start broad (listing repositories and versions) and then drill down.
- It should prefer specific graph queries over broad ones to keep context small.

---

## Graph Query Tools

These tools are exposed to the LLM. Each maps to one or more Cypher queries.

### `list_repositories`

Returns all known repository names and their versions.

```cypher
MATCH (r:Repository)-[:HAS_VERSION]->(v:Version {ingested: true})
RETURN r.name AS repo, collect(v.tag) AS versions
ORDER BY r.name
```

### `search_symbols`

Full-text search for functions or classes by name fragment across a repo/version.

Parameters: `query: String`, `repo?: String`, `version?: String`, `kind?: "function" | "class" | "any"`

```cypher
CALL db.index.fulltext.queryNodes("symbol_names", $query)
YIELD node, score
WHERE ($repo    IS NULL OR node.repo    = $repo)
  AND ($version IS NULL OR node.version = $version)
RETURN node.repo, node.version, node.file, node.name,
       node.start_line, node.end_line, score
ORDER BY score DESC LIMIT 20
```

### `get_symbol_source`

Returns the stored source text of a specific function or class.

Parameters: `repo: String`, `version: String`, `file: String`, `name: String`

### `get_file_symbols`

Lists all symbols defined in a file (without source text).

Parameters: `repo: String`, `version: String`, `file: String`

### `find_callers`

Returns all functions that call the given function within a version.

Parameters: `repo: String`, `version: String`, `function_name: String`

```cypher
MATCH (caller:Function)-[c:CALLS]->(callee:Function {repo: $repo, version: $version, name: $function_name})
WHERE caller.repo = $repo AND caller.version = $version
RETURN caller.file, caller.name, caller.start_line, c.line AS call_site_line
```

### `find_callees`

Returns all functions that the given function calls.

Parameters: `repo: String`, `version: String`, `file: String`, `function_name: String`

### `get_imports`

Returns all import declarations for a file.

Parameters: `repo: String`, `version: String`, `file: String`

### `compare_symbol_across_versions`

Returns the source text of a symbol for two versions side-by-side — useful for "what changed in X between v1 and v2" queries.

Parameters: `repo: String`, `version_a: String`, `version_b: String`, `file: String`, `name: String`

### `run_cypher` *(power tool)*

Executes an arbitrary **read-only** Cypher query composed by the LLM. The driver connection is opened with `AccessMode::Read` so writes are rejected at the protocol level.

Parameters: `query: String`, `params?: Object`

This tool lets the LLM express complex traversals that the fixed tools don't cover (e.g., "find all classes that implement trait X and are imported by module Y").

---

## Configuration Reference

```toml
[server]
host = "0.0.0.0"
port = 8080

[neo4j]
uri      = "bolt://localhost:7687"
user     = "neo4j"
password = "${NEO4J_PASSWORD}"

[llm]
provider      = "anthropic"           # "anthropic" | "openai-compat"
model         = "claude-sonnet-4-6"
api_key       = "${ANTHROPIC_API_KEY}"
# base_url    = "..."                 # only for openai-compat
max_iterations = 20                   # agentic loop cap
```

---

## Source Citation Format

The LLM is instructed to embed citations inline using:

```
[repo-name:v1.2.3:src/path/to/file.rs:42]
```

The server post-processes the final answer with a regex to extract these citations into the structured `sources` array. The raw answer text is returned as-is (citations included) so clients can render them as links.

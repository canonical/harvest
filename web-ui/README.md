# Harvest Web UI

A single-page application for [knowledge-server](../knowledge-server/). It provides three views: a streaming chat interface, an interactive symbol graph explorer, and a Diataxis documentation browser.

## Features

**Chat**
- **Step timeline** — tool invocations appear as a collapsible step-by-step timeline with AI-generated plain-English descriptions, tool name, raw inputs, and result preview
- **Inline symbol graphs** — answers that reference specific symbols embed a mini interactive graph showing that symbol's relationships (calls, contains, inherits, etc.)
- **Markdown answers** — rendered with syntax-highlighted code blocks and copy-to-clipboard buttons
- **Source citations** — inline `[repo:version:file:line]` markers become amber chips that link to the source file; a sources panel lists them all

**Explore**
- **Interactive symbol graph** — browse the full call and relationship graph for any `(repo, version)` pair, rendered with [Cytoscape.js](https://cytoscape.org/) and an off-thread fcose layout
- **Symbol search** — full-text search highlights matching nodes; AI search mode finds semantically related symbols via the `/query/stream` endpoint with live progress on the canvas
- **Source panel** — click any node to see its signature and full source inline

**Document**
- **Diataxis browser** — read AI-generated documentation organised into Tutorials, How-to Guides, Explanations, and Reference sections for any ingested version

**Shell**
- **Dark / light / auto theme** — toggle in the sidebar; persists via `localStorage`; auto follows OS preference with no flash on reload
- **Responsive navigation** — Vanilla Framework `l-application` shell with collapsible sidebar for mobile

---

## Prerequisites

| Tool | Purpose | Install |
|------|---------|---------|
| Node.js ≥ 20 | Run dev server and tests | [nodejs.org](https://nodejs.org) or `nvm install 22` |
| npm ≥ 10 | Package management | bundled with Node.js |
| knowledge-server | Backend API | see [dev-setup](../documentation/developer/dev-setup.md) |

---

## Quick start

```bash
cd web-ui
npm install
npm run dev
# Open http://localhost:5173
```

Vite's dev server proxies all API paths to `http://localhost:8080`:

| Proxied path | Endpoint |
|---|---|
| `/query` | `POST /query` |
| `/query/stream` | `POST /query/stream` |
| `/repositories` | `GET /repositories` |
| `/graph` | `GET /graph/:repo/:version[/source]` |
| `/docs` | `GET /docs/:repo/:version[/:section/:file]` |
| `/tool-description` | `POST /tool-description` |
| `/health` | `GET /health` |

---

## Scripts

| Script | Description |
|--------|-------------|
| `npm run dev` | Start dev server with HMR on port 5173 |
| `npm run build` | Build optimised bundle into `dist/` |
| `npm run preview` | Serve the production build locally |
| `npm test` | Run all unit tests once (CI mode) |
| `npm run test:watch` | Run tests in watch mode |

---

## Project structure

```
web-ui/
├── index.html              Entry HTML — navigation shell, page containers
├── package.json
├── vite.config.js          Vite config with proxy rules and Vitest settings
├── src/
│   ├── api.js              API client: queryStream(), fetchGraph(), fetchDocIndex(), etc.
│   ├── chat.js             Immutable chat state machine
│   ├── documentation.js    Documentation page state machine and renderer
│   ├── format.js           JSON-to-HTML and preview formatters for tool call detail
│   ├── graph-utils.js      Cytoscape node colour/shape helpers and shared stylesheet
│   ├── inline-graph.js     Inline mini-graph renderer mounted inside chat answers
│   ├── layout-worker.js    Web Worker: runs fcose layout off the main thread
│   ├── main.js             App entry point: render loop, routing, event handlers
│   ├── markdown.js         Markdown rendering with citation chip injection
│   ├── repositories.js     Explore page: Cytoscape graph, search, source panel
│   ├── source-panel.js     Shared slide-in source panel component
│   ├── theme.js            Dark/light/auto theme management
│   └── utils.js            escapeHtml, copyText, addCopyButtons
├── src/style.css           Component and layout styles (Vanilla CSS + overrides)
└── tests/
    ├── api.test.js
    ├── chat.test.js
    ├── documentation.test.js
    ├── format.test.js
    ├── graph-utils.test.js
    ├── inline-graph.test.js
    ├── markdown.test.js
    └── utils.test.js
```

---

## Architecture

### Page routing

The app renders three pages inside a Vanilla Framework `l-application` shell. Navigation links use `data-page` attributes; the active page's `<div>` has `hidden` removed.

```
#app-sidebar nav links (data-page="chat"|"repositories"|"documentation")
       │
       ▼
show/hide .page elements:
  #page-chat           ← Chat view
  #page-repositories   ← Explore / graph view
  #page-documentation  ← Document / Diataxis view
```

### Chat streaming flow

```
User submits query
       │
       ▼
queryStream() → POST /query/stream (SSE)
       │
       ├── tool_call   → addToolCall(state)  + fetchToolDescription() → re-render
       ├── tool_result → completeToolCall(state)                       → re-render
       └── done        → finalizeAssistantMessage(state)               → re-render
                              └── mountInlineGraphs(messagesEl)
```

Each `tool_call` event also triggers a `POST /tool-description` request to obtain an AI-generated description for the step-timeline label. Descriptions arrive asynchronously and trigger an additional re-render when ready.

### Chat state machine (`chat.js`)

Chat state is immutable — every function returns a new object:

```
createChatState()
  → addUserMessage(state, text)
  → startAssistantMessage(state)
    → addToolCall(state, {name, input})
    → updateToolCallDescription(state, {id, description})
    → completeToolCall(state, {name, preview})
    → finalizeAssistantMessage(state, {answer, sources, tool_calls_made})
    OR
    → setError(state, message)
```

### Explore page (`repositories.js`)

1. User selects a repo and version → `GET /graph/:repo/:version`
2. Layout computed off-thread by `layout-worker.js` (fcose via Cytoscape.js)
3. Cytoscape renders nodes and edges; colour and shape are derived from `kind` via `graph-utils.js`
4. Symbol search: full-text mode highlights matching nodes instantly; AI mode sends a query over SSE and overlays progress on the canvas, highlighting matched nodes as results arrive
5. Clicking a node → `GET /graph/:repo/:version/source?file=...&name=...` → source panel

### Inline graphs (`inline-graph.js`)

The server embeds `<div class="inline-graph" data-graph="...">` markers in chat answers when it references specific symbols. `mountInlineGraphs()` scans the chat DOM after each render, parses the URL-encoded JSON graph definition from `data-graph`, and mounts a small Cytoscape instance in place.

### Documentation page (`documentation.js`)

State machine with the same immutable pattern as chat. On version select:

1. `GET /docs/:repo/:version` → populate section tree
2. On page select → `GET /docs/:repo/:version/:section/:filename` → render markdown

---

## Tests

Tests are written with [Vitest](https://vitest.dev/) and run in a jsdom environment.

```bash
npm test            # run once
npm run test:watch  # watch mode
npx vitest run --coverage
```

Test coverage:

| File | What is tested |
|------|---------------|
| `api.test.js` | SSE event parsing, error handling, HTTP status codes |
| `chat.test.js` | State transitions, immutability, multi-turn conversations |
| `documentation.test.js` | Doc state machine, section toggling, page selection |
| `format.test.js` | JSON-to-HTML formatting, preview rendering |
| `graph-utils.test.js` | Node colour/shape mappings, stylesheet generation |
| `inline-graph.test.js` | Graph definition parsing, mount/error paths |
| `markdown.test.js` | Markdown rendering, citation extraction, XSS prevention |
| `utils.test.js` | escapeHtml, copyText, addCopyButtons |

---

## Production build

```bash
npm run build
# dist/ contains index.html + hashed assets

# Serve with any static file server:
npx serve dist
```

The knowledge-server can also serve the `dist/` directory directly — configure a static file route pointing at the build output.

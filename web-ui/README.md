# Harvest Web UI

A chat interface for the [knowledge-server](../knowledge-server/). Ask natural-language questions about your ingested codebases and see live tool calls as the agent reasons about your code.

## Features

- **Streaming tool calls** — tool invocations appear as collapsible cards in real time, showing the tool name, inputs, and a result preview
- **Markdown answers** — final answers render as formatted Markdown with syntax-highlighted code blocks
- **Source citations** — inline `[repo:version:file:line]` citations are rendered as highlighted chips with a sources list below each answer
- **Repository sidebar** — lists all ingested repositories and their versions from the live server
- **Vanilla CSS** — uses [Canonical's Vanilla framework](https://vanillaframework.io/) for consistent Ubuntu-style design

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

Vite's dev server proxies `/query`, `/query/stream`, `/repositories`, and `/health` to `http://localhost:8080` automatically. Make sure the knowledge-server is running first.

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
├── index.html          Entry HTML — chat layout, sidebar, input bar
├── package.json
├── vite.config.js      Vite config with proxy rules and Vitest settings
├── src/
│   ├── api.js          API client: queryStream() and queryOnce()
│   ├── chat.js         Immutable chat state machine
│   ├── markdown.js     Markdown rendering with citation highlighting
│   ├── main.js         UI entry point: render loop and event handlers
│   └── style.css       Layout and component styles (Vanilla CSS + overrides)
└── tests/
    ├── api.test.js     Tests for the API client
    ├── chat.test.js    Tests for the chat state machine
    └── markdown.test.js Tests for markdown processing and citation parsing
```

---

## Architecture

### Streaming flow

```
User submits query
       │
       ▼
queryStream() → POST /query/stream (SSE)
       │
       ├── tool_call event  → addToolCall(state, ...)   → re-render
       ├── tool_result event → completeToolCall(state, ...) → re-render
       └── done event       → finalizeAssistantMessage(state, ...) → re-render
```

Each SSE event from the server carries a JSON payload:

| Event type | Payload |
|-----------|---------|
| `tool_call` | `{type, name, input}` |
| `tool_result` | `{type, name, preview}` |
| `done` | `{type, answer, sources, tool_calls_made}` |
| `error` | `{type, message}` |

### State machine (`chat.js`)

Chat state is immutable. Every function returns a new state object:

```
createChatState()
  → addUserMessage(state, text)
  → startAssistantMessage(state)
    → addToolCall(state, {name, input})
    → completeToolCall(state, {name, preview})
    → finalizeAssistantMessage(state, {answer, sources, tool_calls_made})
    OR
    → setError(state, message)
```

### Markdown rendering (`markdown.js`)

Uses [`marked`](https://marked.js.org/) with [`marked-highlight`](https://github.com/markedjs/marked-highlight) and [highlight.js](https://highlightjs.org/). Citation brackets are transformed into `<span class="citation">` elements before markdown parsing, preserving their inline position in the rendered text.

---

## API

The web UI communicates with the knowledge-server over two endpoints:

### `POST /query/stream` (SSE)

Streaming query endpoint. The web UI sends:

```json
{ "query": "How does authentication work?" }
```

The server streams SSE events until a `done` or `error` event is received.

### `GET /repositories`

Returns ingested repositories for the sidebar.

---

## Tests

Tests are written with [Vitest](https://vitest.dev/) and run in a jsdom environment (no browser needed).

```bash
npm test            # run once
npm run test:watch  # watch mode
```

Coverage report:

```bash
npx vitest run --coverage
```

Tests are co-located in `tests/` and cover:
- API client (11 tests): SSE event parsing, error handling, HTTP status codes
- Chat state (16 tests): state transitions, immutability, multi-turn conversations
- Markdown (15 tests): rendering, citation extraction, XSS prevention

---

## Production build

```bash
npm run build
# dist/ contains index.html + hashed assets

# Serve with any static file server:
npx serve dist
# or let the knowledge-server serve the dist/ directory (see server docs)
```

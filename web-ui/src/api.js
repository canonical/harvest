const QUERY_STREAM_URL = '/query/stream';
const QUERY_URL = '/query';
const REPOSITORIES_URL = '/repositories';
const TOOL_DESCRIPTION_URL = '/tool-description';
const GRAPH_URL = '/graph';
const DOCS_URL = '/docs';

/**
 * Send a query to the knowledge server using SSE streaming.
 * Calls `onEvent` for each parsed event as it arrives.
 * Resolves when the stream closes, rejects on HTTP error.
 *
 * @param {string} query
 * @param {(event: object) => void} onEvent
 */
export async function queryStream(query, onEvent) {
  const response = await fetch(QUERY_STREAM_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ query }),
  });

  if (!response.ok) {
    throw new Error(`Server error: ${response.status}`);
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';

  while (true) {
    const { value, done } = await reader.read();
    if (done) break;

    buffer += decoder.decode(value, { stream: true });

    // Process complete SSE events (terminated by blank line)
    const events = buffer.split('\n\n');
    buffer = events.pop() ?? '';

    for (const block of events) {
      for (const line of block.split('\n')) {
        if (!line.startsWith('data: ')) continue;
        const raw = line.slice(6).trim();
        try {
          onEvent(JSON.parse(raw));
        } catch {
          // Skip malformed JSON lines
        }
      }
    }
  }
}

/**
 * Send a query to the knowledge server without streaming.
 * Returns the full JSON response.
 *
 * @param {string} query
 * @returns {Promise<{answer: string, sources: object[], tool_calls_made: number}>}
 */
export async function queryOnce(query) {
  const response = await fetch(QUERY_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ query }),
  });

  if (!response.ok) {
    throw new Error(`Server error: ${response.status}`);
  }

  return response.json();
}

/**
 * Ask the server's LLM for a short description of what a specific tool call
 * is doing. Returns null on any error so callers can fall back gracefully.
 *
 * @param {string} name  Tool name
 * @param {object} input Tool input parameters
 * @returns {Promise<string|null>}
 */
export async function fetchToolDescription(name, input) {
  try {
    const response = await fetch(TOOL_DESCRIPTION_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name, input }),
    });
    if (!response.ok) return null;
    const data = await response.json();
    return data.description ?? null;
  } catch {
    return null;
  }
}

/**
 * Fetch the list of ingested repositories.
 * Returns an empty array on failure instead of throwing.
 *
 * @returns {Promise<{name: string, url?: string, versions: string[]}[]>}
 */
export async function fetchRepositories() {
  try {
    const response = await fetch(REPOSITORIES_URL);
    if (!response.ok) return [];
    return response.json();
  } catch {
    return [];
  }
}

/**
 * Fetch the symbol graph for a repository version.
 *
 * @param {string} repo
 * @param {string} version
 * @returns {Promise<{nodes: object[], edges: object[], truncated: boolean}>}
 */
export async function fetchGraph(repo, version) {
  const url = `${GRAPH_URL}/${encodeURIComponent(repo)}/${encodeURIComponent(version)}`;
  const response = await fetch(url);
  if (!response.ok) throw new Error(`Server error ${response.status}`);
  return response.json();
}

/**
 * Fetch the source code for a specific symbol.
 * Returns null when the symbol is not found.
 *
 * @param {string} repo
 * @param {string} version
 * @param {string} file
 * @param {string} name
 * @returns {Promise<{name, file, kind, start_line, end_line, signature, source}|null>}
 */
export async function fetchSymbolSource(repo, version, file, name) {
  const params = new URLSearchParams({ file, name });
  const url = `${GRAPH_URL}/${encodeURIComponent(repo)}/${encodeURIComponent(version)}/source?${params}`;
  const response = await fetch(url);
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`Server error ${response.status}`);
  return response.json();
}

/**
 * Fetch the documentation index for a repository version.
 * Returns null when documentation has not been generated yet.
 *
 * @param {string} repo
 * @param {string} version
 * @returns {Promise<{repo, version, sections: {tutorials, 'how-to-guides', explanations, reference}}|null>}
 */
export async function fetchDocIndex(repo, version) {
  const url = `${DOCS_URL}/${encodeURIComponent(repo)}/${encodeURIComponent(version)}`;
  const response = await fetch(url);
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`Server error ${response.status}`);
  return response.json();
}

/**
 * Fetch the markdown content of a documentation page.
 * Returns null when the page is not found.
 *
 * @param {string} repo
 * @param {string} version
 * @param {string} section  One of: tutorials, how-to-guides, explanations, reference
 * @param {string} filename Markdown filename (e.g. "getting-started.md")
 * @returns {Promise<string|null>}
 */
export async function fetchDocPage(repo, version, section, filename) {
  const url = `${DOCS_URL}/${encodeURIComponent(repo)}/${encodeURIComponent(version)}/${encodeURIComponent(section)}/${encodeURIComponent(filename)}`;
  const response = await fetch(url);
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`Server error ${response.status}`);
  return response.text();
}

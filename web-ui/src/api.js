const QUERY_STREAM_URL = '/query/stream';
const QUERY_URL = '/query';
const REPOSITORIES_URL = '/repositories';
const TOOL_DESCRIPTION_URL = '/tool-description';
const GRAPH_URL = '/graph';
const DOCS_URL = '/docs';

let _onUnauthorized = null;
export function setUnauthorizedHandler(fn) { _onUnauthorized = fn; }

function handleUnauthorized(status) {
  if (status === 401 && _onUnauthorized) _onUnauthorized();
}

export async function queryStream(query, onEvent) {
  const response = await fetch(QUERY_STREAM_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ query }),
  });

  if (!response.ok) {
    handleUnauthorized(response.status);
    throw new Error(`Server error: ${response.status}`);
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';

  while (true) {
    const { value, done } = await reader.read();
    if (done) break;

    buffer += decoder.decode(value, { stream: true });

    const events = buffer.split('\n\n');
    buffer = events.pop() ?? '';

    for (const block of events) {
      for (const line of block.split('\n')) {
        if (!line.startsWith('data: ')) continue;
        const raw = line.slice(6).trim();
        try {
          onEvent(JSON.parse(raw));
        } catch {
        }
      }
    }
  }
}

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

export async function fetchRepositories() {
  try {
    const response = await fetch(REPOSITORIES_URL);
    if (!response.ok) return [];
    return response.json();
  } catch {
    return [];
  }
}

export async function fetchGraph(repo, version) {
  const url = `${GRAPH_URL}/${encodeURIComponent(repo)}/${encodeURIComponent(version)}`;
  const response = await fetch(url);
  if (!response.ok) throw new Error(`Server error ${response.status}`);
  return response.json();
}

export async function fetchSymbolSource(repo, version, file, name) {
  const params = new URLSearchParams({ file, name });
  const url = `${GRAPH_URL}/${encodeURIComponent(repo)}/${encodeURIComponent(version)}/source?${params}`;
  const response = await fetch(url);
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`Server error ${response.status}`);
  return response.json();
}

export async function fetchDocIndex(repo, version) {
  const url = `${DOCS_URL}/${encodeURIComponent(repo)}/${encodeURIComponent(version)}`;
  const response = await fetch(url);
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`Server error ${response.status}`);
  return response.json();
}

export async function fetchDocPage(repo, version, section, filename) {
  const url = `${DOCS_URL}/${encodeURIComponent(repo)}/${encodeURIComponent(version)}/${encodeURIComponent(section)}/${encodeURIComponent(filename)}`;
  const response = await fetch(url);
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`Server error ${response.status}`);
  return response.text();
}

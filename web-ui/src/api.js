const QUERY_STREAM_URL = '/query/stream';
const QUERY_URL = '/query';
const REPOSITORIES_URL = '/repositories';
const TOOL_DESCRIPTION_URL = '/tool-description';
const GRAPH_URL = '/graph';
const DOCS_URL = '/docs';
const PROJECTS_URL = '/projects';

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

async function projectFetch(url, options = {}) {
  const res = await fetch(url, {
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `Request failed (${res.status})`);
  }
  return res.json();
}

const pid = (id) => `${PROJECTS_URL}/${encodeURIComponent(id)}`;
const cid = (projectId, convId) =>
  `${PROJECTS_URL}/${encodeURIComponent(projectId)}/conversations/${encodeURIComponent(convId)}`;

export const fetchMyGroups      = ()         => projectFetch('/groups');
export const fetchProjects      = ()         => projectFetch(PROJECTS_URL);
export const createProject      = (body)     => projectFetch(PROJECTS_URL, { method: 'POST', body: JSON.stringify(body) });
export const getProject         = (id)       => projectFetch(pid(id));
export const updateProject      = (id, body) => projectFetch(pid(id), { method: 'PUT',  body: JSON.stringify(body) });
export const deleteProject      = (id)       => projectFetch(pid(id), { method: 'DELETE' });

export const listProjectConversations   = (projectId)           => projectFetch(`${pid(projectId)}/conversations`);
export const createProjectConversation  = (projectId, body)     => projectFetch(`${pid(projectId)}/conversations`, { method: 'POST', body: JSON.stringify(body) });
export const getProjectConversation     = (projectId, convId)   => projectFetch(cid(projectId, convId));
export const updateProjectConversation  = (projectId, convId, body) => projectFetch(cid(projectId, convId), { method: 'PUT',  body: JSON.stringify(body) });
export const deleteProjectConversation  = (projectId, convId)   => projectFetch(cid(projectId, convId), { method: 'DELETE' });

export async function projectQueryStream(projectId, query, onEvent) {
  const url = `${pid(projectId)}/query/stream`;
  const response = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ query }),
  });
  if (!response.ok) {
    handleUnauthorized(response.status);
    throw new Error(`Server error: ${response.status}`);
  }

  const reader  = response.body.getReader();
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
        try { onEvent(JSON.parse(line.slice(6).trim())); } catch {}
      }
    }
  }
}

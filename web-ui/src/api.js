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

async function consumeSseStream(response, onEvent) {
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
        try { onEvent(JSON.parse(line.slice(6).trim())); } catch {}
      }
    }
  }
}

export async function queryStream(query, conversationId, attachments, onEvent) {
  const response = await fetch(QUERY_STREAM_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ query, conversation_id: conversationId, attachments }),
  });

  if (!response.ok) {
    handleUnauthorized(response.status);
    throw new Error(`Server error: ${response.status}`);
  }

  await consumeSseStream(response, onEvent);
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

const projectUrl = (id) => `${PROJECTS_URL}/${encodeURIComponent(id)}`;
const conversationUrl = (projectId, convId) =>
  `${PROJECTS_URL}/${encodeURIComponent(projectId)}/conversations/${encodeURIComponent(convId)}`;

export const fetchMyGroups      = ()         => projectFetch('/groups');
export const fetchProjects      = ()         => projectFetch(PROJECTS_URL);
export const createProject      = (body)     => projectFetch(PROJECTS_URL, { method: 'POST', body: JSON.stringify(body) });
export const getProject         = (id)       => projectFetch(projectUrl(id));
export const updateProject      = (id, body) => projectFetch(projectUrl(id), { method: 'PUT',  body: JSON.stringify(body) });
export const deleteProject      = (id)       => projectFetch(projectUrl(id), { method: 'DELETE' });

export const listProjectConversations   = (projectId)           => projectFetch(`${projectUrl(projectId)}/conversations`);
export const createProjectConversation  = (projectId, body)     => projectFetch(`${projectUrl(projectId)}/conversations`, { method: 'POST', body: JSON.stringify(body) });
export const getProjectConversation     = (projectId, convId)   => projectFetch(conversationUrl(projectId, convId));
export const updateProjectConversation  = (projectId, convId, body) => projectFetch(conversationUrl(projectId, convId), { method: 'PUT',  body: JSON.stringify(body) });
export const deleteProjectConversation  = (projectId, convId)   => projectFetch(conversationUrl(projectId, convId), { method: 'DELETE' });

export async function projectQueryStart(projectId, query, conversationId, attachments) {
  const url = `${projectUrl(projectId)}/query/stream`;
  const response = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ query, conversation_id: conversationId, attachments }),
  });
  if (!response.ok) {
    handleUnauthorized(response.status);
    const data = await response.json().catch(() => ({}));
    const e = new Error(data.error || `Server error: ${response.status}`);
    e.status = response.status;
    throw e;
  }
}

export function openProjectEvents(projectId, convId, onEvent) {
  const base = `${projectUrl(projectId)}/events`;
  const url  = convId ? `${base}?conv=${encodeURIComponent(convId)}` : base;
  const es   = new EventSource(url);
  es.onmessage = (e) => {
    try { onEvent(JSON.parse(e.data)); } catch {}
  };
  return es;
}

export async function projectQueryStream(projectId, query, onEvent) {
  const url = `${projectUrl(projectId)}/query/stream`;
  const response = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ query }),
  });
  if (!response.ok) {
    handleUnauthorized(response.status);
    throw new Error(`Server error: ${response.status}`);
  }
  await consumeSseStream(response, onEvent);
}

export async function listProjectAgents(projectId) {
  const response = await fetch(`${projectUrl(projectId)}/agents`);
  handleUnauthorized(response.status);
  if (!response.ok) throw new Error(`Server error: ${response.status}`);
  return response.json();
}

export async function executeAgentCommand(projectId, agentId, command, timeoutSecs = 30) {
  const response = await fetch(`${projectUrl(projectId)}/agents/${agentId}/execute`, {
    method:  'POST',
    headers: { 'Content-Type': 'application/json' },
    body:    JSON.stringify({ command, timeout_secs: timeoutSecs }),
  });
  handleUnauthorized(response.status);
  if (!response.ok) throw new Error(`Server error: ${response.status}`);
  return response.json();
}

export async function deleteAgent(projectId, agentId) {
  return projectFetch(`${projectUrl(projectId)}/agents/${encodeURIComponent(agentId)}`, { method: 'DELETE' });
}

export async function rotateInstallToken(projectId) {
  const response = await fetch(`${projectUrl(projectId)}/agents/rotate-install-token`, {
    method: 'POST',
  });
  handleUnauthorized(response.status);
  if (!response.ok) throw new Error(`Server error: ${response.status}`);
  return response.json();
}

export const fetchProjectOverview      = (projectId) => projectFetch(`${projectUrl(projectId)}/overview`);
export const regenerateProjectOverview = (projectId) => projectFetch(`${projectUrl(projectId)}/overview/regenerate`, { method: 'POST' });

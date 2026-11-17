import { streamQuery } from './api.js';

async function codeFetch(url, options = {}) {
  const response = await fetch(url, {
    ...options,
    headers: { 'Content-Type': 'application/json', ...(options.headers ?? {}) },
  });
  if (response.status === 204) return null;
  const data = await response.json().catch(() => ({}));
  if (!response.ok) {
    const error = new Error(data.error || `Request failed (${response.status})`);
    error.status = response.status;
    throw error;
  }
  return data;
}

const send = (method, body) => ({ method, body: JSON.stringify(body) });

export const listCodeProjects = () => codeFetch('/code/projects');
export const listCodeSessions = () => codeFetch('/code/sessions');

const repositoryUrl = (name, resource, params = {}) => {
  const query = new URLSearchParams(Object.entries(params).filter(([, value]) => value));
  return `/code/repositories/${encodeURIComponent(name)}/${resource}${query.size ? `?${query}` : ''}`;
};
export const listRepositoryVersions = (name) => codeFetch(repositoryUrl(name, 'versions'));
export const listRepositoryTree = (name, version, path) => codeFetch(repositoryUrl(name, 'tree', { version, path }));
export const getRepositoryFile = (name, version, path) => codeFetch(repositoryUrl(name, 'file', { version, path }));
export const repositoryRawUrl = (name, version, path) => repositoryUrl(name, 'raw', { version, path });
export const openCodeProject = (url) => codeFetch('/code/projects', send('POST', { url }));
export const getCodeProject = (id) => codeFetch(`/code/projects/${id}`);
export const saveSetupScript = (id, script) => codeFetch(`/code/projects/${id}/setup`, send('PUT', { script }));
export const saveProjectChecks = (id, checks) => codeFetch(`/code/projects/${id}/checks`, send('PUT', { checks }));
export const listSessionJobs = (id) => codeFetch(`/code/sessions/${id}/jobs`);
export const runSessionChecks = (id) => codeFetch(`/code/sessions/${id}/checks`, { method: 'POST' });
export const getJob = (id) => codeFetch(`/code/jobs/${id}`);
export const stopJob = (id) => codeFetch(`/code/jobs/${id}/stop`, { method: 'POST' });
export const openCodeSession = (projectId, branch, baseBranch) =>
  codeFetch(`/code/projects/${projectId}/sessions`, send('POST', { branch, base_branch: baseBranch }));
export const getCodeSession = (id) => codeFetch(`/code/sessions/${id}`);
export const closeCodeSession = (id) => codeFetch(`/code/sessions/${id}`, { method: 'DELETE' });
export const getSessionChanges = (id, commit = null) =>
  codeFetch(`/code/sessions/${id}/changes${commit ? `?commit=${encodeURIComponent(commit)}` : ''}`);
export const createSessionChat = (id) => codeFetch(`/code/sessions/${id}/chats`, { method: 'POST' });
export const deleteSessionChat = (id, chatId) =>
  codeFetch(`/code/sessions/${id}/chats/${chatId}`, { method: 'DELETE', keepalive: true });
export const renameSessionChat = (id, chatId, title) =>
  codeFetch(`/code/sessions/${id}/chats/${chatId}`, send('PATCH', { title }));
export const restartPreview = (previewId) => codeFetch(`/code/previews/${previewId}/restart`, { method: 'POST' });
export const previewLog = (previewId) => codeFetch(`/code/previews/${previewId}/log`);
export const keepPreview = (previewId) => codeFetch(`/code/previews/${previewId}/keep`, { method: 'POST' });
export const getSessionLayout = (id) => codeFetch(`/code/sessions/${id}/layout`);
export const saveSessionLayout = (id, layout) => codeFetch(`/code/sessions/${id}/layout`, send('PUT', { layout }));
export const listSessionPreviews = (id) => codeFetch(`/code/sessions/${id}/previews`);
export const createSessionPreview = (id, preview) => codeFetch(`/code/sessions/${id}/previews`, send('POST', preview));
export const deleteSessionPreview = (previewId) => codeFetch(`/code/previews/${previewId}`, { method: 'DELETE' });
export const proposeChange = (id, title, description) =>
  codeFetch(`/code/sessions/${id}/change-request`, send('POST', { title, description }));

const REQUEST_NAMES = {
  github: 'Pull request',
  gitea: 'Pull request',
  gitlab: 'Merge request',
  gerrit: 'Gerrit change',
  launchpad: 'Merge proposal',
};

export function indexSummary(index) {
  if (!index) return 'not indexed';
  if (!index.ingested) return 'being built';
  const at = index.commit ? ` at ${index.commit.slice(0, 12)}` : '';
  const failure = index.error ? `; the last refresh failed: ${index.error}` : '';
  return `${index.version}${at}${failure}`;
}

export function changeRequestName(platform) {
  return REQUEST_NAMES[platform] ?? 'Change request';
}

export function sessionQueryStream(sessionId, request, onEvent) {
  return streamQuery(`/code/sessions/${sessionId}/query/stream`, request, onEvent);
}

export function isOpen(session) {
  return session?.status === 'starting' || session?.status === 'active';
}

export function suggestBranch(text) {
  const slug = (text ?? '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40);
  return slug ? `harvest/${slug}` : '';
}

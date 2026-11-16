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
export const openCodeProject = (url) => codeFetch('/code/projects', send('POST', { url }));
export const getCodeProject = (id) => codeFetch(`/code/projects/${id}`);
export const saveSetupScript = (id, script) => codeFetch(`/code/projects/${id}/setup`, send('PUT', { script }));
export const openCodeSession = (projectId, branch, baseBranch) =>
  codeFetch(`/code/projects/${projectId}/sessions`, send('POST', { branch, base_branch: baseBranch }));
export const getCodeSession = (id) => codeFetch(`/code/sessions/${id}`);
export const closeCodeSession = (id) => codeFetch(`/code/sessions/${id}`, { method: 'DELETE' });
export const getSessionChanges = (id) => codeFetch(`/code/sessions/${id}/changes`);
export const proposeChange = (id, title, description) =>
  codeFetch(`/code/sessions/${id}/change-request`, send('POST', { title, description }));

const REQUEST_NAMES = {
  github: 'Pull request',
  gitea: 'Pull request',
  gitlab: 'Merge request',
  gerrit: 'Gerrit change',
  launchpad: 'Merge proposal',
};

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

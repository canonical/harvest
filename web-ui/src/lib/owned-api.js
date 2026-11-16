import {
  listProjectArtifacts, createProjectArtifact,
  listProjectSkills, getProjectSkill, createProjectSkill, updateProjectSkill, deleteProjectSkill,
} from './api.js';

async function request(url, options = {}) {
  const res = await fetch(url, { headers: { 'Content-Type': 'application/json' }, ...options });
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `Request failed (${res.status})`);
  }
  return res.status === 204 ? null : res.json();
}

const personalArtifacts = {
  list: () => request('/me/artifacts'),
  create: (body) => request('/me/artifacts', { method: 'POST', body: JSON.stringify(body) }),
};

const personalSkills = {
  list: () => request('/me/skills'),
  get: (id) => request(`/me/skills/${encodeURIComponent(id)}`),
  create: (body) => request('/me/skills', { method: 'POST', body: JSON.stringify(body) }),
  update: (id, body) => request(`/me/skills/${encodeURIComponent(id)}`, { method: 'PUT', body: JSON.stringify(body) }),
  remove: (id) => request(`/me/skills/${encodeURIComponent(id)}`, { method: 'DELETE' }),
};

export function artifactCollection(projectId) {
  if (!projectId) return personalArtifacts;
  return {
    list: () => listProjectArtifacts(projectId),
    create: (body) => createProjectArtifact(projectId, body),
  };
}

export function skillCollection(projectId) {
  if (!projectId) return personalSkills;
  return {
    list: () => listProjectSkills(projectId),
    get: (id) => getProjectSkill(projectId, id),
    create: (body) => createProjectSkill(projectId, body),
    update: (id, body) => updateProjectSkill(projectId, id, body),
    remove: (id) => deleteProjectSkill(projectId, id),
  };
}

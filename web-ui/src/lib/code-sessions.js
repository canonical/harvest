export const SESSION_STATES = {
  waiting: 'Needs you',
  working: 'Working',
  idle: 'Idle',
  starting: 'Starting',
  failed: 'Failed',
  closed: 'Closed',
};

export function sessionState(session) {
  if (session.status === 'active') return session.activity ?? 'idle';
  return session.status in SESSION_STATES ? session.status : 'closed';
}

export function filterSessions(sessions, { projectIds = [], includeClosed = false } = {}) {
  const chosen = new Set(projectIds);
  return sessions.filter(
    (session) =>
      (!chosen.size || chosen.has(session.project_id)) && (includeClosed || session.status !== 'closed'),
  );
}

const isIngested = (repository) =>
  repository.ingestion_status === 'completed' || (repository.versions ?? []).length > 0;

export function repositoryChoices(repositories, projects) {
  const lastUsed = new Map(projects.map((project) => [project.name, project.last_used_at]));
  const recency = (choice) => (choice.lastUsedAt ? Date.parse(choice.lastUsedAt) : -Infinity);
  return repositories
    .filter(isIngested)
    .map(({ name, url, visibility }) => ({ name, url, visibility, lastUsedAt: lastUsed.get(name) ?? null }))
    .sort((a, b) => recency(b) - recency(a) || a.name.localeCompare(b.name));
}

export function searchChoices(choices, text) {
  const needle = text.trim().toLowerCase();
  if (!needle) return choices;
  return choices.filter((choice) => `${choice.name} ${choice.url}`.toLowerCase().includes(needle));
}

export function filtersFromQuery(query) {
  return {
    projectIds: [query.project ?? []].flat().filter((id) => typeof id === 'string' && id),
    includeClosed: query.closed === '1',
  };
}

export function queryFromFilters({ projectIds, includeClosed }) {
  return {
    ...(projectIds.length ? { project: projectIds } : {}),
    ...(includeClosed ? { closed: '1' } : {}),
  };
}

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

const CHAT_UNAVAILABLE = {
  starting: 'The session is still starting…',
  failed: 'This session could not start, so it cannot run anything.',
  closed: 'This session has ended. Open a new session to keep working.',
};

export function chatUnavailableReason(status) {
  return CHAT_UNAVAILABLE[status] ?? '';
}

const STILL_PREPARING = /still being prepared/i;
const SERVICE_UNREACHABLE = /error sending request|connection refused|operation timed out|lxd is not reachable/i;

export function failureAdvice(error = '') {
  if (STILL_PREPARING.test(error)) {
    return {
      title: "The project's containers are still being prepared",
      message: 'Open a new session in a few minutes.',
      suggestsSetup: false,
    };
  }
  if (SERVICE_UNREACHABLE.test(error)) {
    return {
      title: 'Harvest could not reach its container service',
      message: 'This is usually temporary, for example right after the server restarts. Open a new session in a minute.',
      suggestsSetup: false,
    };
  }
  return {
    title: 'The session could not start',
    message: "Open a new session to try again. If it fails the same way, check the project's container setup.",
    suggestsSetup: true,
  };
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

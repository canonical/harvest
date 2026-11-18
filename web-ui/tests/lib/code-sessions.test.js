import { describe, it, expect } from 'vitest';
import {
  SESSION_STATES,
  chatUnavailableReason,
  failureAdvice,
  sessionState,
  filterSessions,
  repositoryChoices,
  searchChoices,
  filtersFromQuery,
  queryFromFilters,
} from '../../src/lib/code-sessions.js';

const sessions = [
  { id: 'a', project_id: 'p1', status: 'active', activity: 'working' },
  { id: 'b', project_id: 'p2', status: 'active', activity: null },
  { id: 'c', project_id: 'p1', status: 'closed' },
  { id: 'd', project_id: 'p2', status: 'failed' },
];

const ids = (list) => list.map((session) => session.id);

describe('session state', () => {
  it('shows what the agent is doing in an active session', () => {
    expect(sessionState({ status: 'active', activity: 'working' })).toBe('working');
    expect(sessionState({ status: 'active', activity: 'waiting' })).toBe('waiting');
    expect(sessionState({ status: 'active', activity: null })).toBe('idle');
  });

  it('shows the lifecycle status of a session that is not active', () => {
    expect(sessionState({ status: 'starting' })).toBe('starting');
    expect(sessionState({ status: 'failed' })).toBe('failed');
    expect(sessionState({ status: 'closed', activity: 'working' })).toBe('closed');
  });

  it('labels every state it can return', () => {
    for (const status of ['starting', 'failed', 'closed', 'unknown']) {
      expect(SESSION_STATES[sessionState({ status })]).toBeTruthy();
    }
  });
});

describe('session filters', () => {
  it('hides closed sessions unless asked', () => {
    expect(ids(filterSessions(sessions))).toEqual(['a', 'b', 'd']);
    expect(ids(filterSessions(sessions, { includeClosed: true }))).toEqual(['a', 'b', 'c', 'd']);
  });

  it('keeps only the sessions of the chosen projects', () => {
    expect(ids(filterSessions(sessions, { projectIds: ['p1'] }))).toEqual(['a']);
    expect(ids(filterSessions(sessions, { projectIds: ['p1'], includeClosed: true }))).toEqual(['a', 'c']);
    expect(ids(filterSessions(sessions, { projectIds: ['p1', 'p2'] }))).toEqual(['a', 'b', 'd']);
  });

  it('round-trips through the page query', () => {
    const filters = { projectIds: ['p1', 'p2'], includeClosed: true };
    expect(queryFromFilters(filters)).toEqual({ project: ['p1', 'p2'], closed: '1' });
    expect(filtersFromQuery(queryFromFilters(filters))).toEqual(filters);
  });

  it('reads a single project from the query', () => {
    expect(filtersFromQuery({ project: 'p1' }).projectIds).toEqual(['p1']);
  });

  it('leaves the default filters out of the query', () => {
    expect(queryFromFilters({ projectIds: [], includeClosed: false })).toEqual({});
    expect(filtersFromQuery({})).toEqual({ projectIds: [], includeClosed: false });
  });
});

describe('repository choices', () => {
  const repositories = [
    { name: 'zeta', url: 'https://x/zeta', visibility: 'public', ingestion_status: 'completed', versions: ['main'] },
    { name: 'alpha', url: 'https://x/alpha', visibility: 'private', ingestion_status: 'completed', versions: ['main'] },
    { name: 'beta', url: 'https://x/beta', visibility: 'public', ingestion_status: 'failed', versions: ['main'] },
    { name: 'pending', url: 'https://x/pending', visibility: 'public', ingestion_status: 'running', versions: [] },
    { name: 'recent', url: 'https://x/recent', visibility: 'public', ingestion_status: 'completed', versions: ['main'] },
  ];
  const projects = [
    { name: 'zeta', last_used_at: '2026-11-17T10:00:00Z' },
    { name: 'recent', last_used_at: '2026-11-17T12:00:00Z' },
  ];

  it('lists ingested repositories, most recently used first, then by name', () => {
    expect(repositoryChoices(repositories, projects).map((choice) => choice.name)).toEqual(['recent', 'zeta', 'alpha', 'beta']);
  });

  it('keeps what the modal shows about each repository', () => {
    expect(repositoryChoices(repositories, projects)[2]).toEqual({
      name: 'alpha', url: 'https://x/alpha', visibility: 'private', lastUsedAt: null,
    });
  });

  it('searches names and URLs without regard to case', () => {
    const choices = repositoryChoices(repositories, projects);
    expect(searchChoices(choices, ' ZET ').map((choice) => choice.name)).toEqual(['zeta']);
    expect(searchChoices(choices, 'x/al').map((choice) => choice.name)).toEqual(['alpha']);
    expect(searchChoices(choices, '')).toBe(choices);
  });
});

describe('session availability', () => {
  it('lets people chat only in an active session', () => {
    expect(chatUnavailableReason('active')).toBe('');
    expect(chatUnavailableReason('failed')).toContain('could not start');
    expect(chatUnavailableReason('closed')).toContain('has ended');
  });

  it('advises waiting when the containers are still being prepared', () => {
    expect(failureAdvice("the project's containers are still being prepared")).toMatchObject({ suggestsSetup: false });
  });

  it('advises waiting when the container service was unreachable', () => {
    const advice = failureAdvice('creating LXD container: error sending request for url (https://127.0.0.1:8443/1.0/instances)');
    expect(advice.title).toContain('could not reach');
    expect(advice.suggestsSetup).toBe(false);
  });

  it('points to the container setup for other failures', () => {
    expect(failureAdvice('the setup script failed with exit code 1')).toMatchObject({ suggestsSetup: true });
    expect(failureAdvice()).toMatchObject({ title: 'The session could not start' });
  });
});

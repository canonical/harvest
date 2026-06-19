import { describe, it, expect, vi, beforeEach } from 'vitest';
import { setActivePinia, createPinia } from 'pinia';
import { useProjectStore } from '../../src/stores/project.js';

function mockFetch(responses) {
  let call = 0;
  global.fetch = vi.fn(() => {
    const r = responses[call++] ?? responses.at(-1);
    return Promise.resolve({
      ok: r.ok ?? true,
      status: r.status ?? 200,
      json: () => Promise.resolve(r.body ?? {}),
    });
  });
}

const PROJECTS = [
  { id: 'p1', name: 'Alpha', description: '', group_id: 'g1', group_name: 'Team A' },
  { id: 'p2', name: 'Beta',  description: '', group_id: 'g1', group_name: 'Team A' },
];

describe('useProjectStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
  });

  it('starts with empty projects and null selected', () => {
    const store = useProjectStore();
    expect(store.projects).toEqual([]);
    expect(store.selectedProject).toBeNull();
  });

  it('fetchProjects loads project list', async () => {
    mockFetch([{ body: PROJECTS }]);
    const store = useProjectStore();
    await store.fetchProjects();
    expect(store.projects).toHaveLength(2);
    expect(store.projects[0].name).toBe('Alpha');
  });

  it('selectProject sets selectedProject', () => {
    const store = useProjectStore();
    store.projects = PROJECTS;
    store.selectProject(PROJECTS[0]);
    expect(store.selectedProject).toEqual(PROJECTS[0]);
    expect(store.selectedProjectId).toBe('p1');
  });

  it('clearProject resets selectedProject', () => {
    const store = useProjectStore();
    store.projects = PROJECTS;
    store.selectProject(PROJECTS[0]);
    store.clearProject();
    expect(store.selectedProject).toBeNull();
    expect(store.selectedProjectId).toBeNull();
  });

  it('selectProjectById finds project from list', () => {
    const store = useProjectStore();
    store.projects = PROJECTS;
    store.selectProjectById('p2');
    expect(store.selectedProject?.name).toBe('Beta');
  });

  it('selectProjectById stores pending id if projects not loaded yet', () => {
    const store = useProjectStore();
    store.selectProjectById('p1');
    expect(store.selectedProject).toBeNull();
    store.projects = PROJECTS;
    store._flushPending();
    expect(store.selectedProject?.name).toBe('Alpha');
  });

  it('createProject posts and refreshes list', async () => {
    mockFetch([
      { body: { id: 'p3', name: 'Gamma' } },
      { body: [...PROJECTS, { id: 'p3', name: 'Gamma', description: '', group_id: 'g1', group_name: 'Team A' }] },
    ]);
    const store = useProjectStore();
    const project = await store.createProject({ name: 'Gamma', description: '', group_id: 'g1' });
    expect(project.name).toBe('Gamma');
  });
});

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import SkillsView from '../../src/views/SkillsView.vue';
import { useAuthStore } from '../../src/stores/auth.js';

const GLOBAL_SKILLS = [
  { id: 'g1', name: 'juju', description: 'Juju guide', created_at: new Date().toISOString(), updated_at: new Date().toISOString() },
];
const GLOBAL_DETAIL = { id: 'g1', name: 'juju', description: 'Juju guide', content: 'Global body', created_at: new Date().toISOString(), updated_at: new Date().toISOString() };

const PROJECT_SKILLS = [
  { id: 'p1', name: 'runbook', description: 'Team runbook', created_at: new Date().toISOString(), updated_at: new Date().toISOString() },
];
const PROJECT_DETAIL = { id: 'p1', name: 'runbook', description: 'Team runbook', content: 'Project body', created_at: new Date().toISOString(), updated_at: new Date().toISOString() };

function jsonOk(body) {
  return Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve(body) });
}

function mockFetch({ globalList = GLOBAL_SKILLS, projectList = PROJECT_SKILLS, globalDetail = GLOBAL_DETAIL, projectDetail = PROJECT_DETAIL } = {}) {
  global.fetch = vi.fn((url, options = {}) => {
    const method = options.method || 'GET';
    if (url === '/skills' && method === 'GET') return jsonOk(globalList);
    if (url === '/skills/g1' && method === 'GET') return jsonOk(globalDetail);
    if (url.includes('/projects/proj-1/skills') && !url.includes('/p1') && method === 'GET') return jsonOk(projectList);
    if (url.includes('/projects/proj-1/skills/p1') && method === 'GET') return jsonOk(projectDetail);
    if (method === 'POST' || method === 'PUT') return jsonOk({ ok: true, id: 'new-id', name: 'new', created_at: new Date().toISOString() });
    if (method === 'DELETE') return Promise.resolve({ ok: true, status: 204, json: () => Promise.resolve(null) });
    return jsonOk([]);
  });
}

function mountView(projectId = 'proj-1', role = 'regular') {
  const pinia = createPinia();
  setActivePinia(pinia);
  useAuthStore().user = { role };
  return mount(SkillsView, {
    props: { projectId },
    global: { plugins: [pinia] },
  });
}

describe('SkillsView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('shows no-project state and makes no fetches when projectId is null', () => {
    global.fetch = vi.fn();
    const w = mountView(null);
    expect(w.text()).toContain('project');
    expect(global.fetch).not.toHaveBeenCalled();
  });

  it('defaults to the project tab and shows the project skill list', async () => {
    mockFetch();
    const w = mountView('proj-1', 'regular');
    await flushPromises();
    expect(w.text()).toContain('runbook');
    expect(w.text()).not.toContain('juju');
  });

  it('switching to the global tab shows the global skill list', async () => {
    mockFetch();
    const w = mountView('proj-1', 'regular');
    await flushPromises();
    await w.find('[data-testid="tab-global"]').trigger('click');
    await flushPromises();
    expect(w.text()).toContain('juju');
  });

  it('hides add/edit/delete controls in the global tab for non-admin', async () => {
    mockFetch();
    const w = mountView('proj-1', 'regular');
    await flushPromises();
    await w.find('[data-testid="tab-global"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="add-global-skill"]').exists()).toBe(false);
  });

  it('shows add/edit/delete controls in the global tab for admin', async () => {
    mockFetch();
    const w = mountView('proj-1', 'admin');
    await flushPromises();
    await w.find('[data-testid="tab-global"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="add-global-skill"]').exists()).toBe(true);
  });

  it('always shows add control in the project section regardless of role', async () => {
    mockFetch();
    const w = mountView('proj-1', 'regular');
    await flushPromises();
    expect(w.find('[data-testid="add-project-skill"]').exists()).toBe(true);
  });

  it('opens create modal scoped to project when project add button clicked', async () => {
    mockFetch();
    const w = mountView('proj-1', 'regular');
    await flushPromises();
    await w.find('[data-testid="add-project-skill"]').trigger('click');
    expect(w.find('.modal').exists()).toBe(true);
  });

  it('opens create modal scoped to global when global add button clicked (admin)', async () => {
    mockFetch();
    const w = mountView('proj-1', 'admin');
    await flushPromises();
    await w.find('[data-testid="tab-global"]').trigger('click');
    await flushPromises();
    await w.find('[data-testid="add-global-skill"]').trigger('click');
    expect(w.find('.modal').exists()).toBe(true);
  });

  it('selecting a project skill loads detail and shows edit mode round trip', async () => {
    mockFetch();
    const w = mountView('proj-1', 'regular');
    await flushPromises();
    const item = w.find('[data-testid="skill-item-p1"]');
    expect(item.exists()).toBe(true);
    await item.trigger('click');
    await flushPromises();
    expect(w.text()).toContain('Project body');

    await w.find('[data-testid="edit-skill-p1"]').trigger('click');
    await flushPromises();
    const contentInput = w.find('[data-testid="edit-content-p1"]');
    expect(contentInput.exists()).toBe(true);
    await contentInput.setValue('Updated body');
    await w.find('[data-testid="save-skill-p1"]').trigger('click');
    await flushPromises();

    const putCall = global.fetch.mock.calls.find(([url, opts]) => opts && opts.method === 'PUT');
    expect(putCall).toBeTruthy();
  });

  it('cancel edit discards changes without a network call', async () => {
    mockFetch();
    const w = mountView('proj-1', 'regular');
    await flushPromises();
    await w.find('[data-testid="skill-item-p1"]').trigger('click');
    await flushPromises();
    await w.find('[data-testid="edit-skill-p1"]').trigger('click');
    await flushPromises();
    await w.find('[data-testid="edit-content-p1"]').setValue('Changed but cancelled');

    const callsBeforeCancel = global.fetch.mock.calls.length;
    await w.find('[data-testid="cancel-edit-p1"]').trigger('click');
    await flushPromises();

    expect(global.fetch.mock.calls.length).toBe(callsBeforeCancel);
    expect(w.text()).toContain('Project body');
  });

  it('delete-confirm flow removes the skill from the list', async () => {
    mockFetch();
    const w = mountView('proj-1', 'regular');
    await flushPromises();
    await w.find('[data-testid="skill-item-p1"]').trigger('click');
    await flushPromises();
    await w.find('[data-testid="delete-skill-p1"]').trigger('click');
    await flushPromises();
    expect(w.find('.modal').exists()).toBe(true);
    await w.find('[data-testid="confirm-delete"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="skill-item-p1"]').exists()).toBe(false);
  });

  it('surfaces a create error via a notification block', async () => {
    global.fetch = vi.fn((url, options = {}) => {
      const method = options.method || 'GET';
      if (url === '/skills' && method === 'GET') return jsonOk(GLOBAL_SKILLS);
      if (url.includes('/projects/proj-1/skills') && method === 'GET') return jsonOk(PROJECT_SKILLS);
      if (method === 'POST') return Promise.resolve({ ok: false, status: 409, json: () => Promise.resolve({ error: 'a skill with this name already exists' }) });
      return jsonOk([]);
    });
    const w = mountView('proj-1', 'regular');
    await flushPromises();
    await w.find('[data-testid="add-project-skill"]').trigger('click');
    await w.find('[data-testid="skill-name-input"]').setValue('runbook');
    await w.find('[data-testid="save-create"]').trigger('click');
    await flushPromises();
    expect(w.find('.p-notification--negative').exists()).toBe(true);
    expect(w.text()).toContain('already exists');
  });
});

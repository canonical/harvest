import { describe, it, expect, vi, afterEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import ProjectSettingsView from '../../src/views/ProjectSettingsView.vue';
import { useProjectStore } from '../../src/stores/project.js';

const MEMBERS = [
  { user_id: 'u1', email: 'owner@x.com', name: 'Owner', role: 'owner' },
  { user_id: 'u2', email: 'viewer@x.com', name: 'Viewer', role: 'viewer' },
];

function stubFetch() {
  const calls = [];
  vi.stubGlobal('fetch', vi.fn((url, options = {}) => {
    calls.push({ url, method: options.method ?? 'GET', body: options.body ? JSON.parse(options.body) : null });
    const added = { user_id: 'u3', email: 'new@x.com', name: 'New', role: 'editor' };
    const body = options.method === 'PUT' ? [...MEMBERS, added] : MEMBERS;
    return Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve(body) });
  }));
  return calls;
}

function mountAs(role) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const project = useProjectStore();
  project.projects = [{ id: 'p1', name: 'Shared', description: '', role }];
  project.selectProject(project.projects[0]);
  return mount(ProjectSettingsView, { props: { projectId: 'p1' }, global: { plugins: [pinia] } });
}

afterEach(() => vi.unstubAllGlobals());

describe('ProjectSettingsView', () => {
  it('lets an owner share the project by email and role', async () => {
    const calls = stubFetch();
    const w = mountAs('owner');
    await flushPromises();
    await w.find('input[type="email"]').setValue('new@x.com');
    await w.findAll('button').find(b => b.text() === 'Share').trigger('click');
    await flushPromises();
    expect(calls.find(c => c.method === 'PUT')).toMatchObject({
      url: '/projects/p1/members',
      body: { email: 'new@x.com', role: 'editor' },
    });
    expect(w.text()).toContain('new@x.com');
  });

  it('shows a viewer the members without any way to change them', async () => {
    stubFetch();
    const w = mountAs('viewer');
    await flushPromises();
    expect(w.text()).toContain('viewer@x.com');
    expect(w.find('input[type="email"]').exists()).toBe(false);
    expect(w.findAll('button').some(b => ['Share', 'Remove', 'Delete project'].includes(b.text()))).toBe(false);
    expect(w.find('#project-name').attributes('disabled')).toBeDefined();
  });
});

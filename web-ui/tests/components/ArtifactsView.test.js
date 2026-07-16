import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createRouter, createMemoryHistory } from 'vue-router';

const ARTIFACTS = [
  { id: 'a1', title: 'Deploy report', kind: 'markdown', created_by: 'alice', created_at: new Date().toISOString(), updated_at: new Date().toISOString() },
  { id: 'a2', title: 'Incident PDF',  kind: 'pdf',      created_by: 'assistant', created_at: new Date().toISOString(), updated_at: new Date().toISOString() },
];

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    listProjectArtifacts: vi.fn(),
    getArtifact:          vi.fn(),
    deleteArtifact:       vi.fn(() => Promise.resolve()),
  };
});

import ArtifactsView from '../../src/views/ArtifactsView.vue';
import * as api from '../../src/lib/api.js';

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/artifacts',     component: ArtifactsView },
      { path: '/artifacts/:id', component: ArtifactsView },
    ],
  });
}

async function mountView({ path = '/artifacts', projectId = 'proj-1' } = {}) {
  const router = makeRouter();
  router.push(path);
  await router.isReady();
  const w = mount(ArtifactsView, {
    props: { projectId },
    global: { plugins: [createPinia(), router] },
  });
  await flushPromises();
  return { w, router };
}

describe('ArtifactsView', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    api.listProjectArtifacts.mockResolvedValue(ARTIFACTS);
    api.getArtifact.mockImplementation((id) =>
      Promise.resolve({ ...ARTIFACTS.find(a => a.id === id), content: '# Hello\n\nBody text.' }));
  });

  it('shows no-project state when projectId is null and no artifact is targeted', async () => {
    const { w } = await mountView({ path: '/artifacts', projectId: null });
    expect(w.text()).toContain('project');
  });

  it('renders artifact list after load', async () => {
    const { w } = await mountView();
    expect(w.text()).toContain('Deploy report');
    expect(w.text()).toContain('Incident PDF');
  });

  it('shows empty state when no artifacts', async () => {
    api.listProjectArtifacts.mockResolvedValue([]);
    const { w } = await mountView();
    expect(w.text()).toMatch(/no artifacts/i);
  });

  it('selecting an artifact loads and renders its content', async () => {
    const { w } = await mountView();
    const items = w.findAll('.artifacts-list-item');
    expect(items.length).toBe(2);
    await items[0].trigger('click');
    await flushPromises();
    expect(api.getArtifact).toHaveBeenCalledWith('a1');
    expect(w.text()).toContain('Hello');
    expect(w.text()).toContain('Body text');
  });

  it('shows a working download link for the selected artifact', async () => {
    const { w } = await mountView();
    const items = w.findAll('.artifacts-list-item');
    await items[0].trigger('click');
    await flushPromises();
    const link = w.find('a.artifact-download-btn');
    expect(link.exists()).toBe(true);
    expect(link.attributes('href')).toBe('/artifacts/a1/download');
  });

  it('loads the artifact directly when navigated to /artifacts/:id, independent of projectId', async () => {
    const { w } = await mountView({ path: '/artifacts/a2', projectId: null });
    expect(api.getArtifact).toHaveBeenCalledWith('a2');
    expect(w.text()).toContain('Incident PDF');
    expect(w.text()).toContain('Hello');
  });

  it('deletes an artifact after confirmation', async () => {
    const { w } = await mountView();
    const items = w.findAll('.artifacts-list-item');
    await items[0].trigger('click');
    await flushPromises();
    await w.find('.delete-artifact-btn').trigger('click');
    await w.find('.modal .p-button--negative').trigger('click');
    await flushPromises();
    expect(api.deleteArtifact).toHaveBeenCalledWith('a1');
  });
});

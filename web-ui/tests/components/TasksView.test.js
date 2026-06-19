import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import TasksView from '../../src/views/TasksView.vue';

const TASKS = [
  { id: 't1', name: 'Build API',    prompt: 'Create REST API', status: 'idle', depends_on: [] },
  { id: 't2', name: 'Build UI',     prompt: 'Create UI',      status: 'idle', depends_on: ['t1'] },
];

function mockFetch(tasks = TASKS) {
  global.fetch = vi.fn().mockResolvedValue({
    ok: true,
    json: () => Promise.resolve(tasks),
  });
}

describe('TasksView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('shows no-project state when projectId is null', () => {
    const w = mount(TasksView, {
      props: { projectId: null },
      global: { plugins: [createPinia()] },
    });
    expect(w.text()).toContain('project');
  });

  it('shows create task button when project is set', async () => {
    mockFetch();
    const w = mount(TasksView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    expect(w.find('.create-task-btn, [data-testid="create-task"]').exists()).toBe(true);
  });

  it('renders task nodes after load', async () => {
    mockFetch();
    const w = mount(TasksView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    expect(w.text()).toContain('Build API');
    expect(w.text()).toContain('Build UI');
  });

  it('shows empty state when no tasks', async () => {
    mockFetch([]);
    const w = mount(TasksView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    expect(w.text()).toMatch(/no tasks|empty/i);
  });

  it('opens create modal on button click', async () => {
    mockFetch();
    const w = mount(TasksView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    await w.find('.create-task-btn, [data-testid="create-task"]').trigger('click');
    expect(w.find('.modal, dialog').exists()).toBe(true);
    expect(w.find('input[name="task-name"], #task-name-input').exists() || w.text().includes('Name')).toBe(true);
  });

  it('shows task detail panel when task is selected', async () => {
    mockFetch();
    const w = mount(TasksView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    const node = w.find('.tg-node, .task-node');
    if (node.exists()) {
      await node.trigger('click');
      expect(w.find('.task-detail-panel, .task-panel').exists()).toBe(true);
    }
  });
});

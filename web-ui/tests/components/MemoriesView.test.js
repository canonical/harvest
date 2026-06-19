import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import MemoriesView from '../../src/views/MemoriesView.vue';

const MEMORIES = [
  { id: 'm1', title: 'First memory',  content: 'Body 1', created_at: new Date().toISOString(), updated_at: new Date().toISOString() },
  { id: 'm2', title: 'Second memory', content: 'Body 2', created_at: new Date().toISOString(), updated_at: new Date().toISOString() },
];

function mockFetch(list = MEMORIES, detail = MEMORIES[0]) {
  let call = 0;
  global.fetch = vi.fn(() => {
    call++;
    if (call === 1) return Promise.resolve({ ok: true, json: () => Promise.resolve(list) });
    return Promise.resolve({ ok: true, json: () => Promise.resolve(detail) });
  });
}

describe('MemoriesView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('shows no-project state when projectId is null', () => {
    const w = mount(MemoriesView, {
      props: { projectId: null },
      global: { plugins: [createPinia()] },
    });
    expect(w.text()).toContain('project');
  });

  it('renders memory list after load', async () => {
    mockFetch();
    const w = mount(MemoriesView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    expect(w.text()).toContain('First memory');
    expect(w.text()).toContain('Second memory');
  });

  it('shows empty state when no memories', async () => {
    mockFetch([]);
    const w = mount(MemoriesView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    expect(w.text()).toMatch(/no memories|create one/i);
  });

  it('shows add memory button', async () => {
    mockFetch();
    const w = mount(MemoriesView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    expect(w.find('.add-memory-btn, [data-testid="add-memory"]').exists()).toBe(true);
  });

  it('opens create modal on add button click', async () => {
    mockFetch();
    const w = mount(MemoriesView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    await w.find('.add-memory-btn, [data-testid="add-memory"]').trigger('click');
    expect(w.find('.modal, dialog').exists()).toBe(true);
  });

  it('selects memory and loads detail', async () => {
    let call = 0;
    global.fetch = vi.fn(() => {
      call++;
      if (call === 1) return Promise.resolve({ ok: true, json: () => Promise.resolve(MEMORIES) });
      return Promise.resolve({ ok: true, json: () => Promise.resolve(MEMORIES[0]) });
    });
    const w = mount(MemoriesView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    const items = w.findAll('.memories-list-item, .memory-item');
    expect(items.length).toBeGreaterThan(0);
    await items[0].trigger('click');
    await flushPromises();
    expect(w.text()).toContain('First memory');
  });
});

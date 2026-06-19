import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import AgentsView from '../../src/views/AgentsView.vue';

const MOCK_AGENTS = [
  { id: 'ag-1', hostname: 'box1', online: true,  last_seen: new Date().toISOString(), version: '1.0' },
  { id: 'ag-2', hostname: 'box2', online: false, last_seen: new Date().toISOString(), version: '1.0' },
];

function mockApi(agents = MOCK_AGENTS) {
  global.fetch = vi.fn().mockResolvedValue({
    ok: true,
    json: () => Promise.resolve(agents),
  });
}

describe('AgentsView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('renders agent rows after load', async () => {
    mockApi();
    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    const rows = w.findAll('tbody tr');
    expect(rows).toHaveLength(2);
  });

  it('shows online status dot', async () => {
    mockApi();
    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    expect(w.find('.agent-status--online').exists()).toBe(true);
  });

  it('shows offline status', async () => {
    mockApi();
    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    expect(w.find('.agent-status--offline').exists()).toBe(true);
  });

  it('shows empty state when no agents', async () => {
    mockApi([]);
    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    expect(w.text()).toContain('No agents');
  });

  it('shows install modal when button clicked', async () => {
    global.fetch = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_AGENTS) })
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({ token: 'tok123' }) });

    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    await w.find('#install-agent-btn').trigger('click');
    await flushPromises();
    expect(w.find('#install-modal').exists()).toBe(true);
  });
});

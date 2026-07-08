import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises, DOMWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import AgentsView from '../../src/views/AgentsView.vue';
import { useAuthStore } from '../../src/stores/auth.js';

const MOCK_AGENTS = [
  { id: 'ag-1', hostname: 'box1', online: true,  last_seen: new Date().toISOString(), version: '1.0' },
  { id: 'ag-2', hostname: 'box2', online: false, last_seen: new Date().toISOString(), version: '1.0' },
];

const MOCK_FLAVORS = [
  { id: 'tiny',        label: 'Tiny',        cpu: 1, memory_mib: 512 },
  { id: 'small',       label: 'Small',       cpu: 1, memory_mib: 1024 },
  { id: 'medium',      label: 'Medium',      cpu: 2, memory_mib: 2048 },
  { id: 'large',       label: 'Large',       cpu: 4, memory_mib: 4096 },
  { id: 'extra-large', label: 'Extra Large', cpu: 8, memory_mib: 8192 },
];

function mockApi(agents = MOCK_AGENTS) {
  global.fetch = vi.fn().mockResolvedValue({
    ok: true,
    json: () => Promise.resolve(agents),
  });
}

function sseResponse(events) {
  const bytes = new TextEncoder().encode(events.map(e => `data: ${JSON.stringify(e)}\n\n`).join(''));
  let sent = false;
  return {
    ok: true,
    status: 200,
    body: {
      getReader() {
        return {
          read() {
            if (sent) return Promise.resolve({ done: true, value: undefined });
            sent = true;
            return Promise.resolve({ done: false, value: bytes });
          },
          releaseLock() {},
        };
      },
    },
  };
}

function mountWithLxd(agents = MOCK_AGENTS) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useAuthStore().features.lxd = true;
  mockApi(agents);
  return mount(AgentsView, {
    props: { projectId: 'proj-1' },
    global: { plugins: [pinia] },
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

  it('shows a console link per agent pointing at its console route', async () => {
    mockApi();
    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    const links = w.findAll('a.console-icon-btn');
    expect(links).toHaveLength(2);
    expect(links[0].attributes('href')).toBe('/agents/ag-1/console');
    expect(links[1].attributes('href')).toBe('/agents/ag-2/console');
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

  it('does not show install modal directly when LXD is configured', async () => {
    const w = mountWithLxd();
    await flushPromises();
    await w.find('#install-agent-btn').trigger('click');
    await flushPromises();
    expect(w.find('#install-modal').exists()).toBe(false);
    expect(w.find('#agent-choice-modal').exists()).toBe(true);
  });

  it('choice modal offers manual and managed options', async () => {
    const w = mountWithLxd();
    await flushPromises();
    await w.find('#install-agent-btn').trigger('click');
    await flushPromises();
    expect(w.find('#choice-manual-btn').exists()).toBe(true);
    expect(w.find('#choice-lxd-btn').exists()).toBe(true);
  });

  it('choosing manual install opens the existing install modal', async () => {
    const w = mountWithLxd();
    await flushPromises();
    await w.find('#install-agent-btn').trigger('click');
    await flushPromises();
    await w.find('#choice-manual-btn').trigger('click');
    await flushPromises();
    expect(w.find('#agent-choice-modal').exists()).toBe(false);
    expect(w.find('#install-modal').exists()).toBe(true);
  });

  it('choosing managed agent opens the managed-agent modal and loads flavors', async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    useAuthStore().features.lxd = true;
    global.fetch = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_AGENTS) })
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_FLAVORS) });

    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [pinia] },
    });
    await flushPromises();
    await w.find('#install-agent-btn').trigger('click');
    await flushPromises();
    await w.find('#choice-lxd-btn').trigger('click');
    await flushPromises();

    expect(w.find('#managed-agent-modal').exists()).toBe(true);
    expect(w.find('#flavor-select-toggle').text()).toContain('Small');
  });

  it('selecting a flavor updates the toggle button', async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    useAuthStore().features.lxd = true;
    global.fetch = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_AGENTS) })
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_FLAVORS) });

    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [pinia] },
    });
    await flushPromises();
    await w.find('#install-agent-btn').trigger('click');
    await flushPromises();
    await w.find('#choice-lxd-btn').trigger('click');
    await flushPromises();

    await w.find('#flavor-select-toggle').trigger('click');
    await flushPromises();
    await new DOMWrapper(document.body).find('#flavor-option-large').trigger('click');
    await flushPromises();

    expect(w.find('#flavor-select-toggle').text()).toContain('Large');
    expect(w.find('#flavor-select-toggle').text()).toContain('4 vCPU');
  });

  it('submitting the managed-agent form streams live progress through all steps', async () => {
    vi.useFakeTimers();
    try {
      const pinia = createPinia();
      setActivePinia(pinia);
      useAuthStore().features.lxd = true;
      global.fetch = vi.fn()
        .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_AGENTS) })
        .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_FLAVORS) })
        .mockResolvedValueOnce(sseResponse([
          { type: 'phase_start', phase: 'ensure_network' },
          { type: 'phase_start', phase: 'install_token' },
          { type: 'phase_start', phase: 'create_container' },
          { type: 'phase_start', phase: 'start_container' },
          { type: 'phase_start', phase: 'wait_running' },
          { type: 'phase_start', phase: 'install_agent' },
          { type: 'done', hostname: 'agent-abcd' },
        ]))
        .mockResolvedValue({ ok: true, json: () => Promise.resolve(MOCK_AGENTS) });

      const w = mount(AgentsView, {
        props: { projectId: 'proj-1' },
        global: { plugins: [pinia] },
      });
      await flushPromises();
      await w.find('#install-agent-btn').trigger('click');
      await flushPromises();
      await w.find('#choice-lxd-btn').trigger('click');
      await flushPromises();

      await w.find('#managed-agent-name').setValue('build-runner');
      await w.find('#create-managed-agent-btn').trigger('click');
      await flushPromises();

      const createCall = global.fetch.mock.calls.find(([url]) => url.includes('/agents/lxd'));
      expect(createCall).toBeTruthy();
      const body = JSON.parse(createCall[1].body);
      expect(body.name).toBe('build-runner');
      expect(body.flavor).toBe('small');

      expect(w.find('#provision-steps').exists()).toBe(true);
      expect(w.findAll('.lxd-step--done')).toHaveLength(6);
    } finally {
      vi.useRealTimers();
    }
  });

  it('closes the managed-agent modal and shows the new agent once it appears after provisioning', async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    useAuthStore().features.lxd = true;
    global.fetch = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_AGENTS) })
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_FLAVORS) })
      .mockResolvedValueOnce(sseResponse([
        { type: 'done', hostname: 'agent-abcd' },
      ]))
      .mockResolvedValue({
        ok: true,
        json: () => Promise.resolve([
          ...MOCK_AGENTS,
          { id: 'ag-abcd', hostname: 'agent-abcd', online: true, last_seen: new Date().toISOString(), provider: 'lxd' },
        ]),
      });

    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [pinia] },
    });
    await flushPromises();
    await w.find('#install-agent-btn').trigger('click');
    await flushPromises();
    await w.find('#choice-lxd-btn').trigger('click');
    await flushPromises();

    await w.find('#managed-agent-name').setValue('build-runner');
    await w.find('#create-managed-agent-btn').trigger('click');
    await flushPromises();
    await flushPromises();

    expect(w.find('#managed-agent-modal').exists()).toBe(false);
    expect(w.text()).toContain('agent-abcd');
  });

  it('gives up waiting and closes the modal anyway if the agent never appears', async () => {
    vi.useFakeTimers();
    try {
      const pinia = createPinia();
      setActivePinia(pinia);
      useAuthStore().features.lxd = true;
      global.fetch = vi.fn()
        .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_AGENTS) })
        .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_FLAVORS) })
        .mockResolvedValueOnce(sseResponse([
          { type: 'done', hostname: 'agent-abcd' },
        ]))
        .mockResolvedValue({ ok: true, json: () => Promise.resolve(MOCK_AGENTS) });

      const w = mount(AgentsView, {
        props: { projectId: 'proj-1' },
        global: { plugins: [pinia] },
      });
      await flushPromises();
      await w.find('#install-agent-btn').trigger('click');
      await flushPromises();
      await w.find('#choice-lxd-btn').trigger('click');
      await flushPromises();

      await w.find('#managed-agent-name').setValue('build-runner');
      await w.find('#create-managed-agent-btn').trigger('click');
      await flushPromises();

      expect(w.find('#managed-agent-modal').exists()).toBe(true);

      for (let i = 0; i < 15; i += 1) {
        await vi.advanceTimersByTimeAsync(1000);
      }

      expect(w.find('#managed-agent-modal').exists()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('shows the failing step and a Try again option when a phase errors', async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    useAuthStore().features.lxd = true;
    global.fetch = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_AGENTS) })
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(MOCK_FLAVORS) })
      .mockResolvedValueOnce(sseResponse([
        { type: 'phase_start', phase: 'ensure_network' },
        { type: 'error', phase: 'ensure_network', message: 'checking LXD network: SSL certificate problem' },
      ]));

    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [pinia] },
    });
    await flushPromises();
    await w.find('#install-agent-btn').trigger('click');
    await flushPromises();
    await w.find('#choice-lxd-btn').trigger('click');
    await flushPromises();

    await w.find('#managed-agent-name').setValue('build-runner');
    await w.find('#create-managed-agent-btn').trigger('click');
    await flushPromises();

    expect(w.findAll('.lxd-step--error')).toHaveLength(1);
    expect(w.text()).toContain('checking LXD network: SSL certificate problem');
    expect(w.text()).toContain('Try again');
  });

  it('shows an LXD badge for LXD-managed agents', async () => {
    mockApi([
      { id: 'ag-1', hostname: 'box1', online: true, last_seen: new Date().toISOString(), provider: 'lxd' },
    ]);
    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    expect(w.find('.agent-provider-badge').exists()).toBe(true);
  });

  it('shows a type icon reflecting each agent\'s provider', async () => {
    mockApi([
      { id: 'ag-1', hostname: 'box1', online: true,  last_seen: new Date().toISOString(), provider: 'lxd' },
      { id: 'ag-2', hostname: 'box2', online: false, last_seen: new Date().toISOString(), provider: 'manual' },
    ]);
    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    const icons = w.findAll('.agent-type-icon');
    expect(icons).toHaveLength(2);
    expect(icons[0].attributes('title')).toBe('Harvest-managed (LXD)');
    expect(icons[1].attributes('title')).toBe('Manually installed');
  });

  it('delete confirmation mentions the LXD container for LXD-managed agents', async () => {
    mockApi([
      { id: 'ag-1', hostname: 'box1', online: true, last_seen: new Date().toISOString(), provider: 'lxd' },
    ]);
    const w = mount(AgentsView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    await w.findAll('.console-icon-btn--danger')[0].trigger('click');
    await flushPromises();
    expect(w.text()).toContain('LXD container');
  });
});

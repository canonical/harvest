import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createRouter, createMemoryHistory } from 'vue-router';

vi.mock('@xterm/xterm', () => {
  const terminalInstances = [];
  class Terminal {
    constructor(opts) {
      this.opts = opts;
      this.cols = 80;
      this.rows = 24;
      this.written = [];
      this._onData = null;
      terminalInstances.push(this);
    }
    loadAddon() {}
    open() {}
    onData(fn) { this._onData = fn; }
    write(data) { this.written.push(data); }
    dispose() {}
  }
  return { Terminal, __terminalInstances: terminalInstances };
});

vi.mock('@xterm/addon-fit', () => {
  class FitAddon { fit() {} }
  return { FitAddon };
});

const socketMock = vi.hoisted(() => ({
  connect: vi.fn(),
  send:    vi.fn(),
  resize:  vi.fn(),
  close:   vi.fn(),
}));
vi.mock('../../src/composables/useConsoleSocket.js', () => ({
  useConsoleSocket: () => socketMock,
}));

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    listProjectAgents: vi.fn(),
    deleteAgent:       vi.fn(() => Promise.resolve({ ok: true })),
    startAgent:        vi.fn(() => Promise.resolve({ ok: true })),
    stopAgent:         vi.fn(() => Promise.resolve({ ok: true })),
    restartAgent:      vi.fn(() => Promise.resolve({ ok: true })),
    consoleSocketUrl:  vi.fn(() => 'ws://test/console'),
  };
});

import AgentConsoleView from '../../src/views/AgentConsoleView.vue';
import * as api from '../../src/lib/api.js';
import { __terminalInstances } from '@xterm/xterm';

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/agents', component: { template: '<div/>' } },
      { path: '/agents/:agentId/console', component: AgentConsoleView },
    ],
  });
}

async function mountView(agentId = 'ag-1', agents = [{ id: 'ag-1', hostname: 'box1', provider: 'manual', online: true }]) {
  api.listProjectAgents.mockResolvedValue(agents);
  const router = makeRouter();
  router.push(`/agents/${agentId}/console`);
  await router.isReady();
  const w = mount(AgentConsoleView, {
    props: { projectId: 'proj-1' },
    global: { plugins: [createPinia(), router] },
  });
  await flushPromises();
  return { w, router };
}

describe('AgentConsoleView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    __terminalInstances.length = 0;
  });

  it('renders the agent hostname in the header', async () => {
    const { w } = await mountView();
    expect(w.find('.console-title').text()).toBe('box1');
  });

  it('back button navigates to /agents', async () => {
    const { w, router } = await mountView();
    const push = vi.spyOn(router, 'push');
    await w.find('#console-back-btn').trigger('click');
    expect(push).toHaveBeenCalledWith('/agents');
  });

  it('disables power/restart for manually-installed agents', async () => {
    const { w } = await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'manual' }]);
    expect(w.find('#console-power-btn').attributes('disabled')).toBeDefined();
    expect(w.find('#console-restart-btn').attributes('disabled')).toBeDefined();
  });

  it('enables power/restart for lxd-managed agents', async () => {
    const { w } = await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: false }]);
    expect(w.find('#console-power-btn').attributes('disabled')).toBeUndefined();
    expect(w.find('#console-restart-btn').attributes('disabled')).toBeUndefined();
  });

  it('shows a start affordance and calls startAgent when the agent is offline', async () => {
    const { w } = await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: false }]);
    expect(w.find('#console-power-btn').attributes('aria-label')).toBe('Start agent');
    await w.find('#console-power-btn').trigger('click');
    await flushPromises();
    expect(api.startAgent).toHaveBeenCalledWith('proj-1', 'ag-1');
    expect(api.stopAgent).not.toHaveBeenCalled();
  });

  it('shows a stop affordance and calls stopAgent when the agent is online', async () => {
    const { w } = await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: true }]);
    expect(w.find('#console-power-btn').attributes('aria-label')).toBe('Stop agent');
    await w.find('#console-power-btn').trigger('click');
    await flushPromises();
    expect(api.stopAgent).toHaveBeenCalledWith('proj-1', 'ag-1');
    expect(api.startAgent).not.toHaveBeenCalled();
  });

  it('calls restartAgent when restart is clicked', async () => {
    const { w } = await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: true }]);
    await w.find('#console-restart-btn').trigger('click');
    await flushPromises();
    expect(api.restartAgent).toHaveBeenCalledWith('proj-1', 'ag-1');
  });

  it('refreshes agent status after a successful lifecycle action', async () => {
    const { w } = await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: false }]);
    api.listProjectAgents.mockResolvedValue([{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: true }]);
    await w.find('#console-power-btn').trigger('click');
    await flushPromises();
    expect(w.find('.console-status').text()).toBe('Online');
  });

  it('shows an error message when a lifecycle action fails', async () => {
    const { w } = await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: false }]);
    api.startAgent.mockRejectedValueOnce(new Error('LXD is not configured on this server'));
    await w.find('#console-power-btn').trigger('click');
    await flushPromises();
    expect(w.find('.console-action-error').text()).toBe('LXD is not configured on this server');
  });

  it('delete confirms then calls the API and navigates back', async () => {
    const { w, router } = await mountView();
    const push = vi.spyOn(router, 'push');
    await w.find('#console-delete-btn').trigger('click');
    expect(w.find('#console-delete-modal').exists()).toBe(true);
    await w.find('#console-delete-confirm-btn').trigger('click');
    await flushPromises();
    expect(api.deleteAgent).toHaveBeenCalledWith('proj-1', 'ag-1');
    expect(push).toHaveBeenCalledWith('/agents');
  });

  it('connects the console socket for the current agent', async () => {
    await mountView();
    expect(socketMock.connect).toHaveBeenCalledWith('ws://test/console', expect.objectContaining({
      onData: expect.any(Function),
      onControl: expect.any(Function),
      onClose: expect.any(Function),
    }));
  });

  it('writes incoming console bytes to the terminal', async () => {
    await mountView();
    const { onData } = socketMock.connect.mock.calls[0][1];
    const bytes = new Uint8Array([104, 105]);
    onData(bytes);
    expect(__terminalInstances.at(-1).written).toContain(bytes);
  });

  it('forwards keystrokes typed into the terminal to the socket', async () => {
    await mountView();
    const terminal = __terminalInstances.at(-1);
    terminal._onData('ls\r');
    expect(socketMock.send).toHaveBeenCalledWith('ls\r');
  });

  it('does not attempt to connect when the agent is offline at mount', async () => {
    await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: false }]);
    expect(socketMock.connect).not.toHaveBeenCalled();
    expect(__terminalInstances.at(-1).written.join('')).toContain('agent is offline');
  });

  it('offers a keypress reconnect after the shell exits while the agent is still online', async () => {
    await mountView();
    const { onControl } = socketMock.connect.mock.calls[0][1];
    onControl({ type: 'exited', code: 0 });
    expect(__terminalInstances.at(-1).written.join('')).toContain('process exited');

    socketMock.connect.mockClear();
    __terminalInstances.at(-1)._onData('\r');
    expect(socketMock.connect).toHaveBeenCalledTimes(1);
  });

  it('does not attempt a keypress reconnect once the agent has gone offline', async () => {
    vi.useFakeTimers();
    try {
      await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: true }]);
      const { onControl, onClose } = socketMock.connect.mock.calls[0][1];
      onControl({ type: 'exited', code: 0 });
      onClose();

      api.listProjectAgents.mockResolvedValue([{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: false }]);
      await vi.advanceTimersByTimeAsync(3000);
      socketMock.connect.mockClear();

      __terminalInstances.at(-1)._onData('\r');
      expect(socketMock.connect).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it('closes the socket immediately when stop succeeds', async () => {
    const { w } = await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: true }]);
    await w.find('#console-power-btn').trigger('click');
    await flushPromises();
    expect(socketMock.close).toHaveBeenCalled();
    expect(__terminalInstances.at(-1).written.join('')).toContain('agent stopped');
  });

  it('closes the socket immediately when restart is triggered', async () => {
    const { w } = await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: true }]);
    await w.find('#console-restart-btn').trigger('click');
    await flushPromises();
    expect(socketMock.close).toHaveBeenCalled();
    expect(__terminalInstances.at(-1).written.join('')).toContain('restarting agent');
  });

  it('auto-reconnects once polling detects the agent back online after a restart', async () => {
    vi.useFakeTimers();
    try {
      const { w } = await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: true }]);

      api.listProjectAgents.mockResolvedValue([{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: false }]);
      await w.find('#console-restart-btn').trigger('click');
      await flushPromises();
      expect(socketMock.close).toHaveBeenCalled();

      socketMock.connect.mockClear();
      api.listProjectAgents.mockResolvedValue([{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: true }]);

      await vi.advanceTimersByTimeAsync(3000);
      expect(socketMock.connect).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('auto-reconnects once polling detects the agent back online after start', async () => {
    vi.useFakeTimers();
    try {
      const { w } = await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: false }]);
      await w.find('#console-power-btn').trigger('click');
      await flushPromises();
      expect(socketMock.connect).not.toHaveBeenCalled();

      api.listProjectAgents.mockResolvedValue([{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: true }]);
      await vi.advanceTimersByTimeAsync(3000);
      expect(socketMock.connect).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not auto-reconnect after an intentional stop even if the agent later reappears online', async () => {
    vi.useFakeTimers();
    try {
      const { w } = await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: true }]);
      await w.find('#console-power-btn').trigger('click');
      await flushPromises();
      socketMock.connect.mockClear();

      api.listProjectAgents.mockResolvedValue([{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: true }]);
      await vi.advanceTimersByTimeAsync(3000);
      expect(socketMock.connect).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it('detects an unexpected disconnect while connected and closes the socket', async () => {
    vi.useFakeTimers();
    try {
      await mountView('ag-1', [{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: true }]);
      api.listProjectAgents.mockResolvedValue([{ id: 'ag-1', hostname: 'box1', provider: 'lxd', online: false }]);

      await vi.advanceTimersByTimeAsync(3000);
      expect(socketMock.close).toHaveBeenCalled();
      expect(__terminalInstances.at(-1).written.join('')).toContain('agent disconnected');
    } finally {
      vi.useRealTimers();
    }
  });
});

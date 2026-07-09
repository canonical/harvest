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
    listProjectAgents:  vi.fn(),
    deleteAgent:        vi.fn(() => Promise.resolve({ ok: true })),
    startAgent:         vi.fn(() => Promise.resolve({ ok: true })),
    stopAgent:          vi.fn(() => Promise.resolve({ ok: true })),
    restartAgent:       vi.fn(() => Promise.resolve({ ok: true })),
    consoleSocketUrl:   vi.fn(() => 'ws://test/console'),
    listPortForwards:   vi.fn(() => Promise.resolve([])),
    createPortForward:  vi.fn(() => Promise.resolve({ id: 'f-new', route_name: 'app', port: 8080, url: 'http://test/agents/ag-1/app' })),
    updatePortForward:  vi.fn(() => Promise.resolve({ id: 'f-1', route_name: 'app', port: 9090, url: 'http://test/agents/ag-1/app' })),
    deletePortForward:  vi.fn(() => Promise.resolve({ ok: true })),
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

  describe('port forwards', () => {
    it('loads port forwards on mount without showing the manager', async () => {
      api.listPortForwards.mockResolvedValue([
        { id: 'f-1', route_name: 'app', port: 8080, url: 'http://test/agents/ag-1/app' },
      ]);
      const { w } = await mountView();
      expect(api.listPortForwards).toHaveBeenCalledWith('proj-1', 'ag-1');
      expect(w.find('#portforward-manager-modal').exists()).toBe(false);
    });

    it('shows a badge on the header button with the port forward count', async () => {
      api.listPortForwards.mockResolvedValue([
        { id: 'f-1', route_name: 'app', port: 8080, url: 'http://test/agents/ag-1/app' },
        { id: 'f-2', route_name: 'other', port: 9090, url: 'http://test/agents/ag-1/other' },
      ]);
      const { w } = await mountView();
      expect(w.find('#console-portforwards-btn .console-icon-btn__badge').text()).toBe('2');
    });

    it('hides the badge when there are no port forwards', async () => {
      api.listPortForwards.mockResolvedValue([]);
      const { w } = await mountView();
      expect(w.find('#console-portforwards-btn .console-icon-btn__badge').exists()).toBe(false);
    });

    it('opens the manager modal listing existing forwards when the header button is clicked', async () => {
      api.listPortForwards.mockResolvedValue([
        { id: 'f-1', route_name: 'app', port: 8080, url: 'http://test/agents/ag-1/app' },
      ]);
      const { w } = await mountView();
      await w.find('#console-portforwards-btn').trigger('click');
      expect(w.find('#portforward-manager-modal').exists()).toBe(true);
      expect(w.find('.p-table').exists()).toBe(true);
      expect(w.text()).toContain('8080');
    });

    it('shows the route as a short /{route} link pointing at the full forward URL', async () => {
      api.listPortForwards.mockResolvedValue([
        { id: 'f-1', route_name: 'app', port: 8080, url: 'http://test/agents/ag-1/app' },
      ]);
      const { w } = await mountView();
      await w.find('#console-portforwards-btn').trigger('click');
      const link = w.find('.p-table a');
      expect(link.text()).toBe('/app');
      expect(link.attributes('href')).toBe('http://test/agents/ag-1/app');
      expect(w.text()).not.toContain('http://test/agents/ag-1/app');
    });

    it('shows an empty state inside the manager when there are no port forwards', async () => {
      api.listPortForwards.mockResolvedValue([]);
      const { w } = await mountView();
      await w.find('#console-portforwards-btn').trigger('click');
      expect(w.find('.portforward-empty').exists()).toBe(true);
    });

    it('closes the manager modal via the close button', async () => {
      const { w } = await mountView();
      await w.find('#console-portforwards-btn').trigger('click');
      await w.find('#portforward-manager-modal .modal-close').trigger('click');
      expect(w.find('#portforward-manager-modal').exists()).toBe(false);
    });

    it('opens the add form with empty fields', async () => {
      const { w } = await mountView();
      await w.find('#console-portforwards-btn').trigger('click');
      await w.find('#portforward-add-btn').trigger('click');
      expect(w.find('#portforward-route-name-input').element.value).toBe('');
    });

    it('creates a port forward and returns to the list, reloaded', async () => {
      const { w } = await mountView();
      await w.find('#console-portforwards-btn').trigger('click');
      await w.find('#portforward-add-btn').trigger('click');
      await w.find('#portforward-route-name-input').setValue('app');
      await w.find('#portforward-port-input').setValue('8080');

      api.listPortForwards.mockResolvedValue([
        { id: 'f-new', route_name: 'app', port: 8080, url: 'http://test/agents/ag-1/app' },
      ]);
      await w.find('#portforward-save-btn').trigger('click');
      await flushPromises();

      expect(api.createPortForward).toHaveBeenCalledWith('proj-1', 'ag-1', { port: 8080, routeName: 'app' });
      expect(w.find('#portforward-route-name-input').exists()).toBe(false);
      expect(w.find('.p-table').exists()).toBe(true);
      expect(api.listPortForwards).toHaveBeenCalledTimes(2);
    });

    it('shows an error and stays on the form when create fails', async () => {
      api.createPortForward.mockRejectedValueOnce(new Error('route_name already exists'));
      const { w } = await mountView();
      await w.find('#console-portforwards-btn').trigger('click');
      await w.find('#portforward-add-btn').trigger('click');
      await w.find('#portforward-route-name-input').setValue('app');
      await w.find('#portforward-port-input').setValue('8080');
      await w.find('#portforward-save-btn').trigger('click');
      await flushPromises();

      expect(w.find('#portforward-route-name-input').exists()).toBe(true);
      expect(w.text()).toContain('route_name already exists');
    });

    it('cancelling the add form returns to the list', async () => {
      const { w } = await mountView();
      await w.find('#console-portforwards-btn').trigger('click');
      await w.find('#portforward-add-btn').trigger('click');
      await w.find('.modal-actions .p-button--base').trigger('click');
      expect(w.find('#portforward-route-name-input').exists()).toBe(false);
      expect(w.find('#portforward-manager-modal').exists()).toBe(true);
    });

    it('opens the edit form pre-filled with the forward being edited', async () => {
      api.listPortForwards.mockResolvedValue([
        { id: 'f-1', route_name: 'app', port: 8080, url: 'http://test/agents/ag-1/app' },
      ]);
      const { w } = await mountView();
      await w.find('#console-portforwards-btn').trigger('click');
      await w.find('.portforward-edit-btn').trigger('click');
      expect(w.find('#portforward-route-name-input').element.value).toBe('app');
      expect(w.find('#portforward-port-input').element.value).toBe('8080');
    });

    it('updates a port forward and returns to the reloaded list', async () => {
      api.listPortForwards.mockResolvedValue([
        { id: 'f-1', route_name: 'app', port: 8080, url: 'http://test/agents/ag-1/app' },
      ]);
      const { w } = await mountView();
      await w.find('#console-portforwards-btn').trigger('click');
      await w.find('.portforward-edit-btn').trigger('click');
      await w.find('#portforward-port-input').setValue('9090');
      await w.find('#portforward-save-btn').trigger('click');
      await flushPromises();

      expect(api.updatePortForward).toHaveBeenCalledWith('proj-1', 'ag-1', 'f-1', { port: 9090, routeName: 'app' });
      expect(w.find('#portforward-route-name-input').exists()).toBe(false);
    });

    it('asks for inline confirmation before deleting, and cancelling keeps the row', async () => {
      api.listPortForwards.mockResolvedValue([
        { id: 'f-1', route_name: 'app', port: 8080, url: 'http://test/agents/ag-1/app' },
      ]);
      const { w } = await mountView();
      await w.find('#console-portforwards-btn').trigger('click');
      await w.find('.portforward-delete-btn').trigger('click');
      expect(w.text()).toContain('Delete?');

      await w.find('.portforward-delete-cancel-btn').trigger('click');
      expect(api.deletePortForward).not.toHaveBeenCalled();
      expect(w.find('.portforward-edit-btn').exists()).toBe(true);
    });

    it('deletes a port forward after inline confirmation and reloads the list', async () => {
      api.listPortForwards.mockResolvedValue([
        { id: 'f-1', route_name: 'app', port: 8080, url: 'http://test/agents/ag-1/app' },
      ]);
      const { w } = await mountView();
      await w.find('#console-portforwards-btn').trigger('click');
      await w.find('.portforward-delete-btn').trigger('click');

      api.listPortForwards.mockResolvedValue([]);
      await w.find('.portforward-delete-confirm-btn').trigger('click');
      await flushPromises();

      expect(api.deletePortForward).toHaveBeenCalledWith('proj-1', 'ag-1', 'f-1');
      expect(w.find('.portforward-empty').exists()).toBe(true);
    });
  });
});

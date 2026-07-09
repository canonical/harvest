import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { setActivePinia, createPinia } from 'pinia';
import { createRouter, createWebHashHistory } from 'vue-router';
import ChatView from '../../src/views/ChatView.vue';
import { useChatStore } from '../../src/stores/chat.js';
import {
  provisionLxdAgent, deleteAgent, listProjectConversations, getProjectConversation, resumeConfirmAction,
  createPortForward, updatePortForward, deletePortForward,
} from '../../src/lib/api.js';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    openProjectEvents: vi.fn((projectId, convId, onEvent) => {
      global.__projectEventCallback = onEvent;
      return { close: vi.fn() };
    }),
    listProjectConversations: vi.fn(() => Promise.resolve([])),
    getProjectConversation:   vi.fn(() => Promise.resolve({ id: 'conv-1', messages: [] })),
    fetchRepositories:        vi.fn(() => Promise.resolve([])),
    provisionLxdAgent:        vi.fn(() => Promise.resolve()),
    deleteAgent:              vi.fn(() => Promise.resolve()),
    createPortForward:        vi.fn(() => Promise.resolve()),
    updatePortForward:        vi.fn(() => Promise.resolve()),
    deletePortForward:        vi.fn(() => Promise.resolve()),
    resumeConfirmAction:      vi.fn(() => Promise.resolve({ ok: true, resumed: true })),
  };
});

function makeRouter() {
  return createRouter({
    history: createWebHashHistory(),
    routes: [{ path: '/', component: ChatView }],
  });
}

function confirmItems(msg) {
  return (msg.chain ?? []).filter(c => c.type === 'confirm_action');
}

function mockFetch(responses) {
  let call = 0;
  global.fetch = vi.fn(() => {
    const r = responses[call++] ?? responses.at(-1);
    return Promise.resolve({
      ok: r.ok ?? true,
      status: r.status ?? 200,
      json: () => Promise.resolve(r.body ?? {}),
      text: () => Promise.resolve(r.text ?? ''),
      body: r.body_stream ?? null,
    });
  });
}

describe('ChatView', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    global.__projectEventCallback = null;
    vi.clearAllMocks();
  });

  it('renders message input and send button', () => {
    const w = mount(ChatView, {
      props: { projectId: null },
      global: { plugins: [createPinia(), makeRouter()] },
    });
    expect(w.find('textarea, input[type="text"]').exists()).toBe(true);
    expect(w.find('button[type="submit"], .send-btn').exists()).toBe(true);
  });

  it('shows empty state when no messages and no project', () => {
    const w = mount(ChatView, {
      props: { projectId: null },
      global: { plugins: [createPinia(), makeRouter()] },
    });
    expect(w.find('.chat-empty-state, .no-project-state').exists()).toBe(true);
  });

  it('send button disabled when input is empty', async () => {
    const w = mount(ChatView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia(), makeRouter()] },
    });
    const btn = w.find('button[type="submit"], .send-btn');
    expect(btn.attributes('disabled') !== undefined || btn.element.disabled).toBe(true);
  });

  it('displays messages from chat store', async () => {
    const pinia = createPinia();
    const w = mount(ChatView, {
      props: { projectId: null },
      global: { plugins: [pinia, makeRouter()] },
    });
    const chat = useChatStore();
    chat.addUserMessage('Hello world');
    await flushPromises();
    expect(w.text()).toContain('Hello world');
  });

  it('shows conversation list when project is set', async () => {
    mockFetch([
      { body: [] },  // listProjectConversations
    ]);
    const w = mount(ChatView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia(), makeRouter()] },
    });
    await flushPromises();
    expect(w.find('.conv-list, .chat-history-panel, .conversation-list').exists()).toBe(true);
  });

  it('new chat button resets state', async () => {
    const pinia = createPinia();
    const w = mount(ChatView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [pinia, makeRouter()] },
    });
    const chat = useChatStore();
    chat.addUserMessage('Test');
    await flushPromises();
    const newBtn = w.find('.chat-new-btn, [data-testid="new-chat"]');
    if (newBtn.exists()) {
      await newBtn.trigger('click');
      expect(chat.messages.length).toBe(0);
    }
  });

  it('has .chat-with-history layout wrapper when project is set', () => {
    const w = mount(ChatView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia(), makeRouter()] },
    });
    expect(w.find('.chat-with-history').exists()).toBe(true);
  });

  it('has .messages container (not .messages-container)', () => {
    const w = mount(ChatView, {
      props: { projectId: null },
      global: { plugins: [createPinia(), makeRouter()] },
    });
    expect(w.find('.messages').exists()).toBe(true);
  });

  it('has .input-area wrapper', () => {
    const w = mount(ChatView, {
      props: { projectId: null },
      global: { plugins: [createPinia(), makeRouter()] },
    });
    expect(w.find('.input-area').exists()).toBe(true);
  });

  it('conversation list uses .chat-history-panel__list class', async () => {
    mockFetch([{ body: [] }]);
    const w = mount(ChatView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [createPinia(), makeRouter()] },
    });
    await flushPromises();
    expect(w.find('.chat-history-panel__list').exists()).toBe(true);
  });

  it('project SSE thinking event adds thinking to the chat chain', async () => {
    const pinia = createPinia();
    mount(ChatView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [pinia, makeRouter()] },
    });
    await flushPromises();

    const chat = useChatStore(pinia);
    chat.startAssistantMessage();

    expect(global.__projectEventCallback).toBeDefined();
    global.__projectEventCallback({ type: 'thinking', text: 'Analysing the codebase…' });

    const last = chat.messages.at(-1);
    expect(last.chain.some(item => item.type === 'thinking' && item.text === 'Analysing the codebase…')).toBe(true);
  });

  it('project SSE thinking_delta event accumulates into a streaming thinking block', async () => {
    const pinia = createPinia();
    mount(ChatView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [pinia, makeRouter()] },
    });
    await flushPromises();

    const chat = useChatStore(pinia);
    chat.startAssistantMessage();

    global.__projectEventCallback({ type: 'thinking_delta', text: 'Let me ' });
    global.__projectEventCallback({ type: 'thinking_delta', text: 'think.' });

    const last = chat.messages.at(-1);
    const item = last.chain.find(c => c.type === 'thinking');
    expect(item).toBeDefined();
    expect(item.text).toBe('Let me think.');
    expect(item.streaming).toBe(true);
  });

  it('project SSE text_delta event accumulates in pendingAnswer', async () => {
    const pinia = createPinia();
    mount(ChatView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [pinia, makeRouter()] },
    });
    await flushPromises();

    const chat = useChatStore(pinia);
    chat.startAssistantMessage();

    global.__projectEventCallback({ type: 'text_delta', text: 'The answer ' });
    global.__projectEventCallback({ type: 'text_delta', text: 'is 42.' });

    const last = chat.messages.at(-1);
    expect(last.pendingAnswer).toBe('The answer is 42.');
  });

  it('project SSE confirm_action event attaches a pending confirm action', async () => {
    const pinia = createPinia();
    mount(ChatView, {
      props: { projectId: 'proj-1' },
      global: { plugins: [pinia, makeRouter()] },
    });
    await flushPromises();

    const chat = useChatStore(pinia);
    chat.startAssistantMessage();

    global.__projectEventCallback({
      type: 'confirm_action',
      conv_id: null,
      id: 'tc1',
      name: 'create_lxd_agent',
      input: { name: 'build-runner', flavor: 'small' },
      description: 'Create a small agent named build-runner',
    });

    const last = chat.messages.at(-1);
    expect(confirmItems(last)[0].name).toBe('create_lxd_agent');
    expect(confirmItems(last)[0].status).toBe('pending');
  });

  describe('confirm/deny handling', () => {
    async function mountWithPendingConfirm(name, input) {
      listProjectConversations.mockResolvedValue([
        { id: 'conv-1', title: 'Chat', updated_at: new Date().toISOString() },
      ]);
      getProjectConversation.mockResolvedValue({ id: 'conv-1', messages: [] });

      const pinia = createPinia();
      const w = mount(ChatView, {
        props: { projectId: 'proj-1' },
        global: { plugins: [pinia, makeRouter()] },
      });
      await flushPromises();
      await w.find('.conv-item').trigger('click');
      await flushPromises();

      const chat = useChatStore(pinia);
      chat.startAssistantMessage();
      chat.addConfirmAction('tc1', name, input, 'description');
      return { w, chat };
    }

    beforeEach(() => {
      provisionLxdAgent.mockReset().mockResolvedValue();
      deleteAgent.mockReset().mockResolvedValue();
      createPortForward.mockReset().mockResolvedValue();
      updatePortForward.mockReset().mockResolvedValue();
      deletePortForward.mockReset().mockResolvedValue();
      resumeConfirmAction.mockClear().mockResolvedValue({ ok: true, resumed: true });
    });

    it('confirming create_lxd_agent calls provisionLxdAgent and marks done', async () => {
      const { w, chat } = await mountWithPendingConfirm('create_lxd_agent', { name: 'build-runner', flavor: 'small' });
      provisionLxdAgent.mockImplementation((projectId, body, onEvent) => {
        onEvent({ type: 'done' });
        return Promise.resolve();
      });

      await w.find('.confirm-actions .p-button--negative').trigger('click');
      await flushPromises();

      expect(provisionLxdAgent).toHaveBeenCalledWith('proj-1', { name: 'build-runner', description: '', flavor: 'small' }, expect.any(Function));
      expect(confirmItems(chat.messages.at(-1))[0].status).toBe('done');
      expect(resumeConfirmAction).toHaveBeenCalledWith('proj-1', 'conv-1', [
        { toolCallId: 'tc1', status: 'done', resultText: "Agent 'build-runner' created." },
      ]);
    });

    it('failed create_lxd_agent marks the action as error', async () => {
      const { w, chat } = await mountWithPendingConfirm('create_lxd_agent', { name: 'build-runner', flavor: 'small' });
      provisionLxdAgent.mockRejectedValue(new Error('boom'));

      await w.find('.confirm-actions .p-button--negative').trigger('click');
      await flushPromises();

      expect(confirmItems(chat.messages.at(-1))[0].status).toBe('error');
      expect(confirmItems(chat.messages.at(-1))[0].resultText).toBe('boom');
      expect(resumeConfirmAction).toHaveBeenCalledWith('proj-1', 'conv-1', [
        { toolCallId: 'tc1', status: 'error', resultText: 'boom' },
      ]);
    });

    it('confirming delete_agent calls deleteAgent and marks done', async () => {
      const { w, chat } = await mountWithPendingConfirm('delete_agent', { agent_id: 'abc' });

      await w.find('.confirm-actions .p-button--negative').trigger('click');
      await flushPromises();

      expect(deleteAgent).toHaveBeenCalledWith('proj-1', 'abc');
      expect(confirmItems(chat.messages.at(-1))[0].status).toBe('done');
      expect(resumeConfirmAction).toHaveBeenCalledWith('proj-1', 'conv-1', [
        { toolCallId: 'tc1', status: 'done', resultText: 'Agent deleted.' },
      ]);
    });

    it('failed delete_agent marks the action as error', async () => {
      const { w, chat } = await mountWithPendingConfirm('delete_agent', { agent_id: 'abc' });
      deleteAgent.mockRejectedValue(new Error('agent not found'));

      await w.find('.confirm-actions .p-button--negative').trigger('click');
      await flushPromises();

      expect(confirmItems(chat.messages.at(-1))[0].status).toBe('error');
      expect(confirmItems(chat.messages.at(-1))[0].resultText).toBe('agent not found');
      expect(resumeConfirmAction).toHaveBeenCalledWith('proj-1', 'conv-1', [
        { toolCallId: 'tc1', status: 'error', resultText: 'agent not found' },
      ]);
    });

    it('denying does not call the agent APIs, marks the action as denied, and resumes with a decline result', async () => {
      const { w, chat } = await mountWithPendingConfirm('delete_agent', { agent_id: 'abc' });

      await w.find('.confirm-actions .p-button--base').trigger('click');
      await flushPromises();

      expect(deleteAgent).not.toHaveBeenCalled();
      expect(confirmItems(chat.messages.at(-1))[0].status).toBe('denied');
      expect(resumeConfirmAction).toHaveBeenCalledWith('proj-1', 'conv-1', [
        { toolCallId: 'tc1', status: 'denied', resultText: 'Cancelled by user.' },
      ]);
    });

    it('confirming create_port_forward calls createPortForward and marks done', async () => {
      const { w, chat } = await mountWithPendingConfirm('create_port_forward', { agent_id: 'abc', port: 8080, route_name: 'app' });

      await w.find('.confirm-actions .p-button--negative').trigger('click');
      await flushPromises();

      expect(createPortForward).toHaveBeenCalledWith('proj-1', 'abc', { port: 8080, routeName: 'app' });
      expect(confirmItems(chat.messages.at(-1))[0].status).toBe('done');
      expect(resumeConfirmAction).toHaveBeenCalledWith('proj-1', 'conv-1', [
        { toolCallId: 'tc1', status: 'done', resultText: "Port forward 'app' created." },
      ]);
    });

    it('failed create_port_forward marks the action as error', async () => {
      const { w, chat } = await mountWithPendingConfirm('create_port_forward', { agent_id: 'abc', port: 8080, route_name: 'app' });
      createPortForward.mockRejectedValue(new Error('route_name already exists'));

      await w.find('.confirm-actions .p-button--negative').trigger('click');
      await flushPromises();

      expect(confirmItems(chat.messages.at(-1))[0].status).toBe('error');
      expect(confirmItems(chat.messages.at(-1))[0].resultText).toBe('route_name already exists');
    });

    it('confirming update_port_forward calls updatePortForward and marks done', async () => {
      const { w, chat } = await mountWithPendingConfirm('update_port_forward', { agent_id: 'abc', forward_id: 'f1', port: 9090 });

      await w.find('.confirm-actions .p-button--negative').trigger('click');
      await flushPromises();

      expect(updatePortForward).toHaveBeenCalledWith('proj-1', 'abc', 'f1', { port: 9090, routeName: undefined });
      expect(confirmItems(chat.messages.at(-1))[0].status).toBe('done');
      expect(resumeConfirmAction).toHaveBeenCalledWith('proj-1', 'conv-1', [
        { toolCallId: 'tc1', status: 'done', resultText: 'Port forward updated.' },
      ]);
    });

    it('failed update_port_forward marks the action as error', async () => {
      const { w, chat } = await mountWithPendingConfirm('update_port_forward', { agent_id: 'abc', forward_id: 'f1', port: 9090 });
      updatePortForward.mockRejectedValue(new Error('port forward not found'));

      await w.find('.confirm-actions .p-button--negative').trigger('click');
      await flushPromises();

      expect(confirmItems(chat.messages.at(-1))[0].status).toBe('error');
      expect(confirmItems(chat.messages.at(-1))[0].resultText).toBe('port forward not found');
    });

    it('confirming delete_port_forward calls deletePortForward and marks done', async () => {
      const { w, chat } = await mountWithPendingConfirm('delete_port_forward', { agent_id: 'abc', forward_id: 'f1' });

      await w.find('.confirm-actions .p-button--negative').trigger('click');
      await flushPromises();

      expect(deletePortForward).toHaveBeenCalledWith('proj-1', 'abc', 'f1');
      expect(confirmItems(chat.messages.at(-1))[0].status).toBe('done');
      expect(resumeConfirmAction).toHaveBeenCalledWith('proj-1', 'conv-1', [
        { toolCallId: 'tc1', status: 'done', resultText: 'Port forward deleted.' },
      ]);
    });

    it('failed delete_port_forward marks the action as error', async () => {
      const { w, chat } = await mountWithPendingConfirm('delete_port_forward', { agent_id: 'abc', forward_id: 'f1' });
      deletePortForward.mockRejectedValue(new Error('port forward not found'));

      await w.find('.confirm-actions .p-button--negative').trigger('click');
      await flushPromises();

      expect(confirmItems(chat.messages.at(-1))[0].status).toBe('error');
      expect(confirmItems(chat.messages.at(-1))[0].resultText).toBe('port forward not found');
    });

    it('approve all runs every pending action in parallel and resumes once with all results', async () => {
      const { w, chat } = await mountWithPendingConfirm('create_lxd_agent', { name: 'build-runner', flavor: 'small' });
      chat.addConfirmAction('tc2', 'delete_agent', { agent_id: 'abc' }, 'description 2');
      provisionLxdAgent.mockImplementation((projectId, body, onEvent) => {
        onEvent({ type: 'done' });
        return Promise.resolve();
      });
      await flushPromises();

      await w.find('.confirm-actions--all button').trigger('click');
      await flushPromises();

      expect(provisionLxdAgent).toHaveBeenCalled();
      expect(deleteAgent).toHaveBeenCalledWith('proj-1', 'abc');
      expect(confirmItems(chat.messages.at(-1))[0].status).toBe('done');
      expect(confirmItems(chat.messages.at(-1))[1].status).toBe('done');
      expect(resumeConfirmAction).toHaveBeenCalledTimes(1);
      expect(resumeConfirmAction).toHaveBeenCalledWith('proj-1', 'conv-1', [
        { toolCallId: 'tc1', status: 'done', resultText: "Agent 'build-runner' created." },
        { toolCallId: 'tc2', status: 'done', resultText: 'Agent deleted.' },
      ]);
    });
  });
});

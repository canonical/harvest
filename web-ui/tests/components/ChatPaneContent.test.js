import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';

let capturedOnEvent = null;
let esClosed = false;

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    openProjectEvents: vi.fn((_projectId, _convId, onEvent) => {
      capturedOnEvent = onEvent;
      esClosed = false;
      return { close() { esClosed = true; } };
    }),
    listProjectConversations: vi.fn(async () => []),
    listConversations:        vi.fn(async () => []),
    getProjectConversation:    vi.fn(async () => ({ messages: [] })),
    getConversation:           vi.fn(async () => ({ messages: [] })),
    createProjectConversation: vi.fn(async () => ({ id: 'conv-new' })),
    createConversation:        vi.fn(async () => ({ id: 'conv-new' })),
    projectQueryStart:         vi.fn(async () => {}),
    queryStream:               vi.fn(async () => {}),
    fetchRepositories:         vi.fn(async () => []),
    fetchLlmProviders:         vi.fn(async () => ({ providers: [] })),
    resumeConfirmAction:       vi.fn(async () => ({ resumed: false })),
    deleteProjectConversation: vi.fn(async () => {}),
    deleteConversation:        vi.fn(async () => {}),
  };
});

vi.mock('../../src/components/chat/ChatMessage.vue', () => ({
  default: {
    name: 'ChatMessage',
    props: ['msg', 'isLast', 'repoUrlMap'],
    emits: ['choice', 'confirm', 'deny', 'confirmAll'],
    template: '<div class="chat-msg-stub" />',
  },
}));
vi.mock('../../src/components/chat/LlmModelPicker.vue', () => ({
  default: { name: 'LlmModelPicker', template: '<div />' },
}));

import ChatPaneContent from '../../src/components/chat/ChatPaneContent.vue';
import { useChatInstance, disposeChatInstance } from '../../src/composables/useChatInstance.js';

function mountPane({ tabId = 'tab-1', conversationId = 'conv-1', projectId = 'proj-1' } = {}) {
  return mount(ChatPaneContent, {
    props: { tabId, conversationId, projectId },
  });
}

describe('ChatPaneContent — catchup replay on reconnect', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    capturedOnEvent = null;
    esClosed = false;
  });

  afterEach(() => {
    disposeChatInstance('tab-1');
  });

  it('does not duplicate the in-flight response when the event stream replays catchup', async () => {
    const tabId = 'tab-1';
    const chat = useChatInstance(tabId);
    chat.addUserMessage('hello', 'Alice', []);
    chat.startAssistantMessage();
    chat.addTextDelta('partial answer');
    expect(chat.messages).toHaveLength(2);
    expect(chat.loading).toBe(true);

    const w = mountPane({ tabId, conversationId: 'conv-1', projectId: 'proj-1' });
    await flushPromises();
    expect(capturedOnEvent).toBeTruthy();

    capturedOnEvent({
      type: 'user_message', conv_id: 'conv-1', query: 'hello', username: 'Alice', attachments: [],
    });
    capturedOnEvent({ type: 'text_delta', conv_id: 'conv-1', text: 'partial answer' });

    const userMsgs = chat.messages.filter(m => m.role === 'user');
    const asstMsgs = chat.messages.filter(m => m.role === 'assistant');
    expect(userMsgs).toHaveLength(1);
    expect(asstMsgs).toHaveLength(1);
    expect(asstMsgs[0].pendingAnswer).toBe('partial answer');

    w.unmount();
  });
});

describe('ChatPaneContent — jump to latest', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    capturedOnEvent = null;
    esClosed = false;
  });

  afterEach(() => {
    disposeChatInstance('tab-1');
  });

  function mockScrollable(el) {
    Object.defineProperty(el, 'scrollHeight', { configurable: true, get: () => 800 });
    Object.defineProperty(el, 'clientHeight', { configurable: true, get: () => 200 });
    Object.defineProperty(el, 'scrollTop', { configurable: true, writable: true, value: 800 });
  }

  it('stays hidden while the user is following the bottom', async () => {
    const w = mountPane();
    await flushPromises();
    mockScrollable(w.find('.messages').element);

    expect(w.find('[data-testid="chat-jump-latest"]').exists()).toBe(false);
  });

  it('appears once the user scrolls away from the bottom', async () => {
    const w = mountPane();
    await flushPromises();
    const el = w.find('.messages').element;
    mockScrollable(el);

    el.scrollTop = 0;
    await w.find('.messages').trigger('scroll');

    expect(w.find('[data-testid="chat-jump-latest"]').exists()).toBe(true);
  });

  it('does not auto-scroll new events while the user has scrolled away', async () => {
    const w = mountPane();
    await flushPromises();
    const el = w.find('.messages').element;
    mockScrollable(el);

    el.scrollTop = 0;
    await w.find('.messages').trigger('scroll');

    capturedOnEvent({ type: 'user_message', conv_id: 'conv-1', query: 'hi', username: 'Alice', attachments: [] });
    capturedOnEvent({ type: 'text_delta', conv_id: 'conv-1', text: 'partial answer' });
    await flushPromises();

    expect(el.scrollTop).toBe(0);
  });

  it('clicking jump-to-latest scrolls to the bottom and hides itself again', async () => {
    const w = mountPane();
    await flushPromises();
    const el = w.find('.messages').element;
    mockScrollable(el);

    el.scrollTop = 0;
    await w.find('.messages').trigger('scroll');
    await w.find('[data-testid="chat-jump-latest"]').trigger('click');

    expect(el.scrollTop).toBe(800);
    expect(w.find('[data-testid="chat-jump-latest"]').exists()).toBe(false);
  });
});

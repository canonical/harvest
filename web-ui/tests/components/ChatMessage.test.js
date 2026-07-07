import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import ChatMessage from '../../src/components/chat/ChatMessage.vue';
import { mountInlineGraphs } from '../../src/lib/inline-graph.js';

vi.mock('../../src/lib/inline-graph.js', () => ({
  mountInlineGraphs: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

const userMsg = { role: 'user', text: 'Hello', attachments: [], username: 'Alice' };

const assistantDone = {
  role: 'assistant', status: 'done',
  chain: [], tool_calls: [], answer: 'World', sources: [], tool_calls_made: 0,
};

const assistantLoading = {
  role: 'assistant', status: 'loading',
  chain: [], tool_calls: [], answer: null, sources: [], tool_calls_made: 0,
};

const assistantWithThinking = {
  role: 'assistant', status: 'done',
  chain: [
    { type: 'thinking', text: 'Let me reason…' },
    { type: 'tool_call', id: 1, name: 'search_symbols', input: { q: 'foo' }, status: 'done', preview: 'found 2', description: 'Searching…' },
  ],
  tool_calls: [],
  answer: 'Here is the answer.', sources: [], tool_calls_made: 1,
};

const assistantWithSources = {
  role: 'assistant', status: 'done',
  chain: [], tool_calls: [],
  answer: 'See [1]',
  sources: [{ repo: 'myrepo', version: 'main', file: 'src/main.rs', line: 10 }],
  tool_calls_made: 0,
};

const assistantError = {
  role: 'assistant', status: 'error',
  chain: [], tool_calls: [], answer: null, error: 'Something went wrong', sources: [],
};

describe('ChatMessage — user', () => {
  it('renders user message text', () => {
    const w = mount(ChatMessage, { props: { msg: userMsg } });
    expect(w.text()).toContain('Hello');
  });

  it('applies message--user class', () => {
    const w = mount(ChatMessage, { props: { msg: userMsg } });
    expect(w.find('.message--user').exists()).toBe(true);
  });
});

describe('ChatMessage — assistant loading', () => {
  it('shows loading indicator when loading and no chain', () => {
    const w = mount(ChatMessage, { props: { msg: assistantLoading } });
    expect(w.find('.loading-dots').exists()).toBe(true);
  });
});

describe('ChatMessage — assistant done', () => {
  it('renders answer text', () => {
    const w = mount(ChatMessage, { props: { msg: assistantDone } });
    expect(w.text()).toContain('World');
  });

  it('applies message--assistant class', () => {
    const w = mount(ChatMessage, { props: { msg: assistantDone } });
    expect(w.find('.message--assistant').exists()).toBe(true);
  });
});

describe('ChatMessage — thinking + tool calls', () => {
  it('renders thinking block', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithThinking } });
    expect(w.find('.thinking-group').exists()).toBe(true);
    expect(w.html()).toContain('Let me reason…');
  });

  it('renders tool call step', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithThinking } });
    expect(w.find('.tc-step').exists()).toBe(true);
  });
});

describe('ChatMessage — sources', () => {
  it('renders source chips', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithSources } });
    expect(w.find('.source-chip').exists()).toBe(true);
  });
});

describe('ChatMessage — error', () => {
  it('renders error message', () => {
    const w = mount(ChatMessage, { props: { msg: assistantError } });
    expect(w.text()).toContain('Something went wrong');
  });
});

describe('ChatMessage — structure', () => {
  it('user message has .message__bubble wrapper', () => {
    const w = mount(ChatMessage, { props: { msg: userMsg } });
    expect(w.find('.message__bubble').exists()).toBe(true);
  });

  it('user message shows .message__sender', () => {
    const w = mount(ChatMessage, { props: { msg: userMsg } });
    expect(w.find('.message__sender').exists()).toBe(true);
  });

  it('assistant answer is inside .message__bubble', () => {
    const w = mount(ChatMessage, { props: { msg: assistantDone } });
    expect(w.find('.message__bubble').exists()).toBe(true);
  });

  it('sources use .source-chips container', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithSources } });
    expect(w.find('.source-chips').exists()).toBe(true);
  });
});

const GRAPH_ANSWER = '```harvest-graph\n{"symbols":[{"name":"Foo","kind":"class","file":"src/foo.rs","start_line":1}],"relations":[]}\n```';

const assistantWithGraph = {
  role: 'assistant', status: 'done',
  chain: [], tool_calls: [],
  answer: GRAPH_ANSWER,
  sources: [], tool_calls_made: 0,
};

const confirmChainItem = (overrides = {}) => ({
  type: 'confirm_action',
  id: 'tc1',
  name: 'create_lxd_agent',
  input: { name: 'build-runner', flavor: 'small' },
  description: 'Create a small agent named build-runner',
  status: 'pending', steps: [], resultText: '',
  ...overrides,
});

const assistantWithConfirmPending = {
  role: 'assistant', status: 'done',
  chain: [confirmChainItem()], tool_calls: [], answer: null, sources: [], tool_calls_made: 1,
};

describe('ChatMessage — confirm_action chain entries', () => {
  it('renders the description text', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithConfirmPending, isLast: true } });
    expect(w.text()).toContain('Create a small agent named build-runner');
  });

  it('shows Confirm/Cancel buttons when pending and isLast', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithConfirmPending, isLast: true } });
    expect(w.find('.confirm-actions').exists()).toBe(true);
    expect(w.text()).toContain('Confirm');
    expect(w.text()).toContain('Cancel');
  });

  it('hides Confirm/Cancel buttons when not isLast', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithConfirmPending, isLast: false } });
    expect(w.find('.confirm-actions').exists()).toBe(false);
  });

  it('emits confirm with the action id when Confirm is clicked', async () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithConfirmPending, isLast: true } });
    await w.find('.confirm-actions .p-button--negative').trigger('click');
    expect(w.emitted('confirm')).toEqual([['tc1']]);
  });

  it('emits deny with the action id when Cancel is clicked', async () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithConfirmPending, isLast: true } });
    await w.find('.confirm-actions .p-button--base').trigger('click');
    expect(w.emitted('deny')).toEqual([['tc1']]);
  });

  it('shows cancelled status text and hides buttons once denied', () => {
    const msg = { ...assistantWithConfirmPending, chain: [confirmChainItem({ status: 'denied' })] };
    const w = mount(ChatMessage, { props: { msg, isLast: true } });
    expect(w.find('.confirm-actions').exists()).toBe(false);
    expect(w.text()).toContain('Cancelled');
  });

  it('shows result text once done', () => {
    const msg = { ...assistantWithConfirmPending, chain: [confirmChainItem({ status: 'done', resultText: "Agent 'build-runner' created." })] };
    const w = mount(ChatMessage, { props: { msg, isLast: true } });
    expect(w.find('.confirm-actions').exists()).toBe(false);
    expect(w.text()).toContain("Agent 'build-runner' created.");
  });

  it('shows result text on error', () => {
    const msg = { ...assistantWithConfirmPending, chain: [confirmChainItem({ status: 'error', resultText: 'LXD is not configured on this server' })] };
    const w = mount(ChatMessage, { props: { msg, isLast: true } });
    expect(w.text()).toContain('LXD is not configured on this server');
  });

  it('renders provision steps when present', () => {
    const msg = {
      ...assistantWithConfirmPending,
      chain: [confirmChainItem({
        status: 'running',
        steps: [{ id: 'ensure_network', label: 'Ensuring network exists', status: 'active', detail: '' }],
      })],
    };
    const w = mount(ChatMessage, { props: { msg, isLast: true } });
    expect(w.find('.lxd-step').exists()).toBe(true);
  });

  it('shows a running status and hides the Confirm/Cancel buttons while creating', () => {
    const msg = { ...assistantWithConfirmPending, chain: [confirmChainItem({ status: 'running', steps: [] })] };
    const w = mount(ChatMessage, { props: { msg, isLast: true } });
    expect(w.find('.confirm-actions').exists()).toBe(false);
    expect(w.text()).toContain('Creating…');
  });

  it('shows a running status while deleting even with no provision steps', () => {
    const msg = {
      ...assistantWithConfirmPending,
      chain: [confirmChainItem({ name: 'delete_agent', status: 'running', steps: [] })],
    };
    const w = mount(ChatMessage, { props: { msg, isLast: true } });
    expect(w.find('.lxd-steps').exists()).toBe(false);
    expect(w.find('.confirm-actions').exists()).toBe(false);
    expect(w.text()).toContain('Deleting…');
  });

  it('does not render confirm block when chain has no confirm_action entries', () => {
    const w = mount(ChatMessage, { props: { msg: assistantDone, isLast: true } });
    expect(w.find('.message__confirm').exists()).toBe(false);
  });

  it('renders a tool call before and after a confirm card in chain order', () => {
    const msg = {
      ...assistantWithConfirmPending,
      chain: [
        { type: 'tool_call', name: 'list_agents', input: {}, status: 'done', preview: '[]' },
        confirmChainItem({ status: 'done', resultText: "Agent 'build-runner' created." }),
        { type: 'tool_call', name: 'run_command', input: {}, status: 'done', preview: 'ok' },
      ],
    };
    const w = mount(ChatMessage, { props: { msg, isLast: true } });
    const kinds = [...w.find('.tc-chain').element.children].map(el =>
      el.classList.contains('tc-step') ? 'tool_call' : 'confirm_action'
    );
    expect(kinds).toEqual(['tool_call', 'confirm_action', 'tool_call']);
  });

  it('does not show Approve all when there is only one pending action', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithConfirmPending, isLast: true } });
    expect(w.text()).not.toContain('Approve all');
  });

  it('shows an Approve all button and one card per action when multiple are pending', () => {
    const msg = {
      ...assistantWithConfirmPending,
      chain: [
        confirmChainItem(),
        confirmChainItem({ id: 'tc2', name: 'delete_agent', input: { agent_id: 'abc' }, description: 'Delete agent abc' }),
      ],
    };
    const w = mount(ChatMessage, { props: { msg, isLast: true } });
    expect(w.findAll('.message__confirm').length).toBe(2);
    expect(w.text()).toContain('Approve all');
  });

  it('hides Approve all once every action has resolved', () => {
    const msg = {
      ...assistantWithConfirmPending,
      chain: [
        confirmChainItem({ status: 'done', resultText: 'done' }),
        confirmChainItem({ id: 'tc2', name: 'delete_agent', input: {}, description: 'Delete agent abc', status: 'denied' }),
      ],
    };
    const w = mount(ChatMessage, { props: { msg, isLast: true } });
    expect(w.text()).not.toContain('Approve all');
  });

  it('emits confirmAll when Approve all is clicked', async () => {
    const msg = {
      ...assistantWithConfirmPending,
      chain: [
        confirmChainItem(),
        confirmChainItem({ id: 'tc2', name: 'delete_agent', input: {}, description: 'Delete agent abc' }),
      ],
    };
    const w = mount(ChatMessage, { props: { msg, isLast: true } });
    await w.find('.confirm-actions--all button').trigger('click');
    expect(w.emitted('confirmAll')).toBeTruthy();
  });
});

describe('ChatMessage — inline graph mounting', () => {
  it('calls mountInlineGraphs on mount when answer is already present (historical chat bug)', () => {
    mount(ChatMessage, { props: { msg: assistantWithGraph } });
    expect(mountInlineGraphs).toHaveBeenCalledOnce();
    expect(mountInlineGraphs).toHaveBeenCalledWith(expect.any(HTMLElement));
  });

  it('calls mountInlineGraphs when answer is set after mount (streaming case)', async () => {
    const w = mount(ChatMessage, { props: { msg: { ...assistantLoading } } });
    expect(mountInlineGraphs).not.toHaveBeenCalled();

    await w.setProps({ msg: { ...assistantWithGraph } });
    await flushPromises();

    expect(mountInlineGraphs).toHaveBeenCalledOnce();
    expect(mountInlineGraphs).toHaveBeenCalledWith(expect.any(HTMLElement));
  });

  it('does not call mountInlineGraphs when answer is null on mount', () => {
    mount(ChatMessage, { props: { msg: assistantLoading } });
    expect(mountInlineGraphs).not.toHaveBeenCalled();
  });
});

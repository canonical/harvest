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
  it('shows the activity line saying the agent is thinking before anything arrives', () => {
    const w = mount(ChatMessage, { props: { msg: assistantLoading } });
    const line = w.find('.activity-line');
    expect(line.exists()).toBe(true);
    expect(line.classes()).toContain('activity-line--working');
    expect(line.text()).toContain('Thinking');
  });
});

describe('ChatMessage — activity line', () => {
  const working = {
    role: 'assistant', status: 'loading', pendingAnswer: '', phase: 'Reading source', intent: 'research',
    chain: [
      { type: 'thinking', text: "I'm reading the driver support matrix.", streaming: false },
      { type: 'tool_call', id: 7, name: 'read_sources', input: { files: ['doc/support-matrix.ini'] }, status: 'running', preview: null },
    ],
    tool_calls: [], answer: null, sources: [], tool_calls_made: 0,
  };

  it('shows one line for the current step and hides the steps', () => {
    const w = mount(ChatMessage, { props: { msg: working } });
    expect(w.find('.activity-line__line').text()).toBe("I'm reading the driver support matrix.");
    expect(w.find('.activity-line__steps').text()).toContain('2 steps');
    expect(w.find('.tc-chain').isVisible()).toBe(false);
  });

  it('opens and closes the steps from the line', async () => {
    const w = mount(ChatMessage, { props: { msg: working }, attachTo: document.body });
    const toggle = w.find('.activity-line__row');
    await toggle.trigger('click');
    expect(toggle.attributes('aria-expanded')).toBe('true');
    expect(w.find('.tc-chain').isVisible()).toBe(true);
    expect(w.find('.tc-step').isVisible()).toBe(true);
    await toggle.trigger('click');
    expect(w.find('.tc-chain').isVisible()).toBe(false);
    w.unmount();
  });

  it('shows the live timer on the activity line instead of the meta row', () => {
    const msg = { ...working, startedAt: Date.now() - 12000 };
    const w = mount(ChatMessage, { props: { msg } });
    expect(w.find('.activity-line__elapsed').text()).toMatch(/1[12]s/);
    expect(w.find('.duration-badge').exists()).toBe(false);
  });

  it('keeps the intent badge and phase label out of the line while loading', () => {
    const w = mount(ChatMessage, { props: { msg: working } });
    expect(w.find('.intent-badge').exists()).toBe(false);
    expect(w.find('.phase-label').exists()).toBe(false);
  });

  it('keeps a short intent sentence out of the answer area', () => {
    const msg = { ...working, chain: [], pendingAnswer: "I'm locating the volume drivers." };
    const w = mount(ChatMessage, { props: { msg } });
    expect(w.find('.message__answer--streaming').exists()).toBe(false);
    expect(w.find('.activity-line__line').text()).toBe("I'm locating the volume drivers.");
  });

  it('streams a long answer in the answer area and says it is writing', () => {
    const msg = { ...working, chain: [], pendingAnswer: 'Cinder ships these drivers:\n- LVM\n- Ceph' };
    const w = mount(ChatMessage, { props: { msg } });
    expect(w.find('.message__answer--streaming').exists()).toBe(true);
    expect(w.find('.activity-line__line').text()).toBe('Writing the answer');
  });

  it('summarises a finished turn and keeps its steps one click away', () => {
    const msg = { ...assistantWithThinking, durationMs: 11000 };
    const w = mount(ChatMessage, { props: { msg } });
    const line = w.find('.activity-line');
    expect(line.classes()).toContain('activity-line--done');
    expect(line.text()).toContain('2 steps');
    expect(w.find('.activity-line__steps').text()).toContain('Show steps');
  });

  it('shows no line on a finished answer without steps', () => {
    const w = mount(ChatMessage, { props: { msg: assistantDone } });
    expect(w.find('.activity-line').exists()).toBe(false);
  });

  it('marks an interrupted turn as stopped', () => {
    const msg = { ...assistantWithThinking, status: 'error', error: 'interrupted' };
    const w = mount(ChatMessage, { props: { msg } });
    expect(w.find('.activity-line').classes()).toContain('activity-line--stopped');
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

describe('ChatMessage — parallel research chain entries', () => {
  const assistantWithParallelResearch = {
    role: 'assistant', status: 'done',
    chain: [
      {
        type: 'parallel_research', id: 1, merging: true, totalDurationMs: 34000,
        leads: [
          { label: 'how OIDC is implemented', status: 'done', iterations: 8, preview: 'OIDC uses pyoidc...', durationMs: 12000 },
          { label: 'how LDAP is implemented', status: 'running', iterations: null, preview: null, durationMs: null },
        ],
      },
      { type: 'tool_call', id: 2, name: 'search_symbols', input: {}, status: 'done', preview: 'ok', description: 'Searching…', gapFill: true },
    ],
    tool_calls: [],
    answer: 'Comparison answer.', sources: [], tool_calls_made: 1,
  };

  it('renders one lane per lead with its question as the label', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithParallelResearch } });
    const lanes = w.findAll('.pr-lane__label');
    expect(lanes).toHaveLength(2);
    expect(lanes[0].text()).toBe('how OIDC is implemented');
    expect(lanes[1].text()).toBe('how LDAP is implemented');
  });

  it('shows duration and round count for a finished lead but not for a running one', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithParallelResearch } });
    const stats = w.findAll('.pr-lane__stats');
    expect(stats).toHaveLength(1);
    expect(stats[0].text()).toContain('8 rounds');
  });

  it('shows the merge header once merging has started', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithParallelResearch } });
    expect(w.find('.pr-block__title').text()).toBe('Investigated 2 leads in parallel · 34s');
    expect(w.find('.pr-block__merge').exists()).toBe(true);
  });

  it('shows the in-progress header while a fan-out has not finished merging yet', () => {
    const stillRunning = {
      ...assistantWithParallelResearch,
      chain: [{ ...assistantWithParallelResearch.chain[0], merging: false, totalDurationMs: null }],
    };
    const w = mount(ChatMessage, { props: { msg: stillRunning } });
    expect(w.find('.pr-block__title').text()).toBe('Investigating 2 leads in parallel');
    expect(w.find('.pr-block__merge').exists()).toBe(false);
  });

  it('expands a lane to show its finding preview on click, only for leads that have one', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithParallelResearch } });
    const rows = w.findAll('.pr-lane__row');
    expect(w.find('.pr-lane__detail').exists()).toBe(false);
    rows[0].trigger('click');
    return flushPromises().then(() => {
      expect(w.find('.pr-lane__detail').text()).toBe('OIDC uses pyoidc...');
    });
  });

  it('tags a tool call made during the merge step as gap-fill', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithParallelResearch } });
    expect(w.find('.tc-step__gap-badge').exists()).toBe(true);
  });
});

describe('ChatMessage — sources', () => {
  it('renders one source location', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithSources } });
    expect(w.find('.source-loc').exists()).toBe(true);
  });

  it('renders a real link with a correct href when the repo URL is known', () => {
    const w = mount(ChatMessage, {
      props: { msg: assistantWithSources, repoUrlMap: { myrepo: 'https://github.com/acme/myrepo' } },
    });
    const loc = w.find('.source-loc');
    expect(loc.element.tagName).toBe('A');
    expect(loc.attributes('href')).toBe('https://github.com/acme/myrepo/blob/main/src/main.rs#L10');
    expect(loc.classes()).not.toContain('source-loc--inert');
  });

  it('degrades to an inert, non-clickable location when the repo URL is unknown', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithSources, repoUrlMap: {} } });
    const loc = w.find('.source-loc');
    expect(loc.element.tagName).toBe('SPAN');
    expect(loc.attributes('href')).toBeUndefined();
    expect(loc.classes()).toContain('source-loc--inert');
  });

  it('links to the bare file with a file-only title when the source has no line number', () => {
    const wholeFile = {
      ...assistantWithSources,
      sources: [{ repo: 'myrepo', version: 'main', file: 'src/main.rs', line: 0 }],
    };
    const w = mount(ChatMessage, {
      props: { msg: wholeFile, repoUrlMap: { myrepo: 'https://github.com/acme/myrepo' } },
    });
    const loc = w.find('.source-loc');
    expect(loc.attributes('href')).toBe('https://github.com/acme/myrepo/blob/main/src/main.rs');
    expect(loc.attributes('title')).toBe('myrepo main · src/main.rs');
    expect(loc.find('.source-loc__line').text()).toBe('file');
  });

  it('builds a range URL and title when the source spans multiple lines', () => {
    const withRange = {
      ...assistantWithSources,
      sources: [{ repo: 'myrepo', version: 'main', file: 'src/main.rs', line: 10, end_line: 20 }],
    };
    const w = mount(ChatMessage, {
      props: { msg: withRange, repoUrlMap: { myrepo: 'https://github.com/acme/myrepo' } },
    });
    const loc = w.find('.source-loc');
    expect(loc.attributes('href')).toBe('https://github.com/acme/myrepo/blob/main/src/main.rs#L10-L20');
    expect(loc.attributes('title')).toBe('myrepo main · src/main.rs:10-20');
    expect(loc.find('.source-loc__line').text()).toBe('10-20');
  });

  it('groups multiple citations to the same file under one file row', () => {
    const sameFile = {
      ...assistantWithSources,
      sources: [
        { repo: 'myrepo', version: 'main', file: 'src/main.rs', line: 10 },
        { repo: 'myrepo', version: 'main', file: 'src/main.rs', line: 40 },
        { repo: 'myrepo', version: 'main', file: 'src/other.rs', line: 5 },
      ],
    };
    const w = mount(ChatMessage, { props: { msg: sameFile } });
    expect(w.findAll('.source-group')).toHaveLength(2);
    expect(w.findAll('.source-loc')).toHaveLength(3);
  });

  it('numbers locations across the whole answer, not per file group', () => {
    const sameFile = {
      ...assistantWithSources,
      sources: [
        { repo: 'myrepo', version: 'main', file: 'src/main.rs', line: 10 },
        { repo: 'myrepo', version: 'main', file: 'src/other.rs', line: 5 },
        { repo: 'myrepo', version: 'main', file: 'src/main.rs', line: 40 },
      ],
    };
    const w = mount(ChatMessage, { props: { msg: sameFile } });
    const nums = w.findAll('.source-loc__num').map(n => n.text());
    expect(nums).toEqual(['1', '3', '2']);
  });

  it('shows the sources section collapsed by default even when there are few sources', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithSources } });
    expect(w.find('.sources-toggle').attributes('aria-expanded')).toBe('false');
    expect(w.find('.source-groups').isVisible()).toBe(false);
  });

  it('shows the sources section collapsed by default when there are many sources', () => {
    const many = {
      ...assistantWithSources,
      sources: Array.from({ length: 8 }, (_, i) => ({ repo: 'myrepo', version: 'main', file: `src/f${i}.rs`, line: 1 })),
    };
    const w = mount(ChatMessage, { props: { msg: many } });
    expect(w.find('.sources-toggle').attributes('aria-expanded')).toBe('false');
    expect(w.find('.source-groups').isVisible()).toBe(false);
    expect(w.find('.sources-toggle__count').text()).toBe('8');
  });

  it('toggles the sources section open on click', async () => {
    const many = {
      ...assistantWithSources,
      sources: Array.from({ length: 8 }, (_, i) => ({ repo: 'myrepo', version: 'main', file: `src/f${i}.rs`, line: 1 })),
    };
    const w = mount(ChatMessage, { props: { msg: many } });
    await w.find('.sources-toggle').trigger('click');
    expect(w.find('.sources-toggle').attributes('aria-expanded')).toBe('true');
    expect(w.find('.source-groups').isVisible()).toBe(true);
  });
});

const assistantWithProvider = {
  role: 'assistant', status: 'done',
  chain: [], tool_calls: [],
  answer: 'Done',
  sources: [], tool_calls_made: 0,
  provider_used: { provider_id: 'anthropic-main', kind: 'anthropic', model: 'claude-sonnet-5' },
};

describe('ChatMessage — run details on the status line', () => {
  const withUsage = {
    ...assistantWithProvider,
    durationMs: 18000,
    llm_call_count: 4,
    cost_microusd: 31900,
    usage: { input_tokens: 49267, output_tokens: 853 },
  };

  it('puts the model, calls, cost and tokens on the status line after the time', () => {
    const w = mount(ChatMessage, { props: { msg: withUsage } });
    const line = w.find('.message__status-bar .activity-line');
    expect(line.find('.activity-line__line').text()).toBe('Worked for 18s');
    expect(line.find('.activity-line__details').text()).toBe('claude-sonnet-5 · 4 calls · $0.0319 · 49,267 in / 853 out');
  });

  it('shows no badges above the status line', () => {
    const w = mount(ChatMessage, { props: { msg: withUsage } });
    expect(w.find('.provider-badge').exists()).toBe(false);
    expect(w.find('.cost-badge').exists()).toBe(false);
    expect(w.find('.duration-badge').exists()).toBe(false);
  });

  it('keeps run details off the line while the answer is still loading', () => {
    const w = mount(ChatMessage, { props: { msg: { ...withUsage, status: 'loading' } } });
    expect(w.find('.activity-line__details').exists()).toBe(false);
  });

  it('leaves the status line out when there is nothing to report', () => {
    const w = mount(ChatMessage, { props: { msg: assistantDone } });
    expect(w.find('.activity-line').exists()).toBe(false);
  });
});

describe('ChatMessage — status bar (intent + phase)', () => {
  const loadingWithIntent = {
    role: 'assistant', status: 'loading',
    chain: [], tool_calls: [],
    intent: 'research', phase: 'Reading source',
    answer: null, pendingAnswer: '', sources: [], tool_calls_made: 0,
  };

  const doneWithIntent = {
    role: 'assistant', status: 'done',
    chain: [], tool_calls: [],
    intent: 'research', phase: null,
    answer: 'Done', sources: [], tool_calls_made: 1,
  };

  it('shows only the activity line in the status bar while loading', () => {
    const w = mount(ChatMessage, { props: { msg: loadingWithIntent } });
    const bar = w.find('.message__status-bar');
    expect(bar.exists()).toBe(true);
    expect(bar.find('.activity-line').exists()).toBe(true);
    expect(bar.find('.intent-badge').exists()).toBe(false);
  });

  it('status bar is sticky while loading', () => {
    const w = mount(ChatMessage, { props: { msg: loadingWithIntent } });
    expect(w.find('.message__status-bar--sticky').exists()).toBe(true);
  });

  it('status bar is not sticky when done', () => {
    const w = mount(ChatMessage, { props: { msg: doneWithIntent } });
    expect(w.find('.message__status-bar--sticky').exists()).toBe(false);
  });

  it('does not show phase label when done', () => {
    const w = mount(ChatMessage, { props: { msg: doneWithIntent } });
    expect(w.find('.phase-label').exists()).toBe(false);
  });

  it('never shows the intent classification as a label', () => {
    const w = mount(ChatMessage, { props: { msg: doneWithIntent } });
    expect(w.find('.intent-badge').exists()).toBe(false);
    expect(w.text()).not.toContain('Researching');
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

  it('assistant answer is not inside a bubble (full-width)', () => {
    const w = mount(ChatMessage, { props: { msg: assistantDone } });
    expect(w.find('.message--assistant .message__bubble').exists()).toBe(false);
  });

  it('sources use .sources-block container', () => {
    const w = mount(ChatMessage, { props: { msg: assistantWithSources } });
    expect(w.find('.sources-block').exists()).toBe(true);
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
    await w.find('.confirm-actions .p-button--positive').trigger('click');
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

  it('shows an Updating… status while a port forward update is running', () => {
    const msg = {
      ...assistantWithConfirmPending,
      chain: [confirmChainItem({ name: 'update_port_forward', status: 'running', steps: [] })],
    };
    const w = mount(ChatMessage, { props: { msg, isLast: true } });
    expect(w.text()).toContain('Updating…');
  });

  it('shows a generic Working… status for an unrecognised confirmable tool name', () => {
    const msg = {
      ...assistantWithConfirmPending,
      chain: [confirmChainItem({ name: 'some_future_tool', status: 'running', steps: [] })],
    };
    const w = mount(ChatMessage, { props: { msg, isLast: true } });
    expect(w.text()).toContain('Working…');
  });

  it('keeps a pending confirmation visible while the other steps stay collapsed', () => {
    const msg = {
      ...assistantWithConfirmPending,
      status: 'loading',
      chain: [
        { type: 'tool_call', name: 'list_agents', input: {}, status: 'done', preview: '[]' },
        confirmChainItem(),
      ],
    };
    const w = mount(ChatMessage, { props: { msg, isLast: true }, attachTo: document.body });
    expect(w.find('.message__confirm').isVisible()).toBe(true);
    expect(w.find('.tc-step').isVisible()).toBe(false);
    expect(w.find('.activity-line').classes()).toContain('activity-line--waiting');
    w.unmount();
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
    const kinds = [...w.find('.tc-chain__list').element.children].map(el =>
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

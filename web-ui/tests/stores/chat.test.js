import { describe, it, expect } from 'vitest';
import { useChatStore } from '../../src/stores/chat.js';

function msgs(store) { return store.messages; }
function last(store) { return store.messages.at(-1); }

describe('initial state', () => {
  it('starts empty', () => {
    const s = useChatStore();
    expect(msgs(s)).toHaveLength(0);
    expect(s.loading).toBe(false);
    expect(s.pendingAttachments).toHaveLength(0);
  });
});

describe('addUserMessage', () => {
  it('appends a user message', () => {
    const s = useChatStore();
    s.addUserMessage('hello', null, []);
    expect(msgs(s)).toHaveLength(1);
    expect(last(s).role).toBe('user');
    expect(last(s).text).toBe('hello');
  });

  it('stores username when provided', () => {
    const s = useChatStore();
    s.addUserMessage('hi', 'Alice', []);
    expect(last(s).username).toBe('Alice');
  });

  it('stores attachments', () => {
    const s = useChatStore();
    s.addUserMessage('hi', null, [{ id: 1, name: 'file.txt' }]);
    expect(last(s).attachments).toHaveLength(1);
  });
});

describe('assistant message lifecycle', () => {
  it('startAssistantMessage creates a loading message', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    expect(last(s).role).toBe('assistant');
    expect(last(s).status).toBe('loading');
    expect(last(s).chain).toEqual([]);
    expect(s.loading).toBe(true);
  });

  it('finalizeAssistantMessage marks done', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.finalizeAssistantMessage({ answer: 'done', sources: [], tool_calls_made: 0 });
    expect(last(s).status).toBe('done');
    expect(last(s).answer).toBe('done');
    expect(s.loading).toBe(false);
  });

  it('startAssistantMessage initializes provider_used to null', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    expect(last(s).provider_used).toBeNull();
  });

  it('finalizeAssistantMessage stores provider_used when present', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.finalizeAssistantMessage({
      answer: 'done', sources: [], tool_calls_made: 0,
      provider_used: { provider_id: 'anthropic-main', kind: 'anthropic', model: 'claude-sonnet-5' },
    });
    expect(last(s).provider_used).toEqual({ provider_id: 'anthropic-main', kind: 'anthropic', model: 'claude-sonnet-5' });
  });

  it('finalizeAssistantMessage leaves provider_used null when absent', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.finalizeAssistantMessage({ answer: 'done', sources: [], tool_calls_made: 0 });
    expect(last(s).provider_used).toBeNull();
  });

  it('setError marks error state', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.setError('something broke');
    expect(last(s).status).toBe('error');
    expect(last(s).error).toBe('something broke');
    expect(s.loading).toBe(false);
  });
});

describe('addThinking', () => {
  it('adds thinking item to chain', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addThinking('Let me think…');
    expect(last(s).chain).toHaveLength(1);
    expect(last(s).chain[0].type).toBe('thinking');
    expect(last(s).chain[0].text).toBe('Let me think…');
    expect(last(s).chain[0].streaming).toBe(false);
  });

  it('does not affect tool_calls array', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addThinking('hmm');
    expect(last(s).tool_calls).toHaveLength(0);
  });

  it('multiple thinking items accumulate', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addThinking('a');
    s.addThinking('b');
    expect(last(s).chain).toHaveLength(2);
  });

  it('closes an active streaming thinking block before adding the consolidated one', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addThinkingDelta('partial...');
    expect(last(s).chain[0].streaming).toBe(true);
    s.addThinking('full reasoning');
    expect(last(s).chain.at(-1).streaming).toBe(false);
    expect(last(s).chain.at(-1).text).toBe('full reasoning');
  });
});

describe('addThinkingDelta', () => {
  it('creates a new streaming thinking item on first delta', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addThinkingDelta('I should');
    const item = last(s).chain[0];
    expect(item.type).toBe('thinking');
    expect(item.text).toBe('I should');
    expect(item.streaming).toBe(true);
  });

  it('appends to the current streaming thinking item', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addThinkingDelta('Hello');
    s.addThinkingDelta(' world');
    expect(last(s).chain).toHaveLength(1);
    expect(last(s).chain[0].text).toBe('Hello world');
  });

  it('creates a new item if the last chain item is not a streaming thinking block', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addToolCall('search', {}, null);
    s.addThinkingDelta('next thought');
    expect(last(s).chain).toHaveLength(2);
    expect(last(s).chain[1].type).toBe('thinking');
  });
});

describe('addTextDelta', () => {
  it('accumulates text in pendingAnswer', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addTextDelta('Hello');
    s.addTextDelta(' world');
    expect(last(s).pendingAnswer).toBe('Hello world');
  });

  it('pendingAnswer starts empty', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    expect(last(s).pendingAnswer).toBe('');
  });

  it('finalizeAssistantMessage clears pendingAnswer', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addTextDelta('partial text');
    s.finalizeAssistantMessage({ answer: 'full answer', sources: [], tool_calls_made: 0 });
    expect(last(s).pendingAnswer).toBe('');
  });

  it('finalizeAssistantMessage falls back to pendingAnswer when answer is null', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addTextDelta('streamed answer');
    s.finalizeAssistantMessage({ answer: null, sources: [], tool_calls_made: 0 });
    expect(last(s).answer).toBe('streamed answer');
  });
});

describe('tool calls', () => {
  it('addToolCall appends to chain and tool_calls', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addToolCall('search_symbols', {}, 'Searching…');
    expect(last(s).chain).toHaveLength(1);
    expect(last(s).chain[0].type).toBe('tool_call');
    expect(last(s).tool_calls).toHaveLength(1);
  });

  it('completeToolCall marks running call done', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addToolCall('search_symbols', {}, null);
    s.completeToolCall('search_symbols', 'found 3 results');
    const tc = last(s).tool_calls[0];
    expect(tc.status).toBe('done');
    expect(tc.preview).toBe('found 3 results');
  });

  it('completes first matching running call, not subsequent', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addToolCall('fn', {}, null);
    s.addToolCall('fn', {}, null);
    s.completeToolCall('fn', 'result');
    expect(last(s).tool_calls[0].status).toBe('done');
    expect(last(s).tool_calls[1].status).toBe('running');
  });

  it('chain preserves thinking → tool_call order', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addThinking('thinking');
    s.addToolCall('tool_a', {}, null);
    s.addThinking('more thinking');
    s.addToolCall('tool_b', {}, null);
    const types = last(s).chain.map(c => c.type);
    expect(types).toEqual(['thinking', 'tool_call', 'thinking', 'tool_call']);
  });

  it('addToolCall promotes streamed preamble text to a Thinking block', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addTextDelta('Let me look that up');
    expect(last(s).pendingAnswer).toBe('Let me look that up');
    s.addToolCall('search', {}, null);
    expect(last(s).pendingAnswer).toBe('');
    expect(last(s).chain).toHaveLength(2);
    expect(last(s).chain[0].type).toBe('thinking');
    expect(last(s).chain[0].text).toBe('Let me look that up');
    expect(last(s).chain[0].streaming).toBe(false);
    expect(last(s).chain[1].type).toBe('tool_call');
  });

  it('addToolCall with no pending text adds only the tool call', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addToolCall('search', {}, null);
    expect(last(s).chain).toHaveLength(1);
    expect(last(s).chain[0].type).toBe('tool_call');
  });

  it('addToolCall closes a streaming ThinkingDelta block before appending tool call', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addThinkingDelta('reasoning...');
    expect(last(s).chain[0].streaming).toBe(true);
    s.addToolCall('search', {}, null);
    expect(last(s).chain[0].streaming).toBe(false);
    expect(last(s).chain[1].type).toBe('tool_call');
  });
});

describe('setQuestion', () => {
  it('attaches question to last assistant message', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.setQuestion('Pick one?', ['a', 'b']);
    expect(last(s).question).toEqual({ question: 'Pick one?', choices: ['a', 'b'] });
  });
});

function confirmItems(s) {
  return last(s).chain.filter(c => c.type === 'confirm_action');
}

describe('addConfirmAction', () => {
  it('appends a pending confirm action to the chain of the last assistant message', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addConfirmAction('tc1', 'create_lxd_agent', { name: 'build-runner', flavor: 'small' }, 'Create a small agent named build-runner');
    expect(confirmItems(s)).toEqual([{
      type: 'confirm_action',
      id: 'tc1',
      name: 'create_lxd_agent',
      input: { name: 'build-runner', flavor: 'small' },
      description: 'Create a small agent named build-runner',
      status: 'pending',
      steps: [],
      resultText: '',
    }]);
  });

  it('appends a second pending action to the chain rather than replacing the first', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addConfirmAction('tc1', 'create_lxd_agent', {}, 'first');
    s.addConfirmAction('tc2', 'delete_agent', {}, 'second');
    expect(confirmItems(s).map(a => a.id)).toEqual(['tc1', 'tc2']);
  });

  it('keeps a tool call before it and a confirm action after in chain order', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addToolCall('list_agents', {});
    s.addConfirmAction('tc1', 'delete_agent', {}, 'desc');
    expect(last(s).chain.map(c => c.type)).toEqual(['tool_call', 'confirm_action']);
  });
});

describe('updateConfirmActionItem', () => {
  it('merges a patch into the matching confirm action in the chain', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addConfirmAction('tc1', 'delete_agent', { agent_id: 'abc' }, 'Delete agent abc');
    s.updateConfirmActionItem('tc1', { status: 'running' });
    expect(confirmItems(s)[0].status).toBe('running');
    expect(confirmItems(s)[0].input).toEqual({ agent_id: 'abc' });
  });

  it('does nothing when there is no matching confirm action on the last message', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    expect(() => s.updateConfirmActionItem('tc1', { status: 'running' })).not.toThrow();
    expect(confirmItems(s)).toEqual([]);
  });

  it('only patches the action with the matching id', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addConfirmAction('tc1', 'create_lxd_agent', {}, 'first');
    s.addConfirmAction('tc2', 'delete_agent', {}, 'second');
    s.updateConfirmActionItem('tc2', { status: 'done' });
    expect(confirmItems(s)[0].status).toBe('pending');
    expect(confirmItems(s)[1].status).toBe('done');
  });

  it('replaces steps array rather than merging elements', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.addConfirmAction('tc1', 'create_lxd_agent', {}, 'desc');
    s.updateConfirmActionItem('tc1', { steps: [{ id: 'ensure_network', status: 'active' }] });
    expect(confirmItems(s)[0].steps).toEqual([{ id: 'ensure_network', status: 'active' }]);
  });
});

describe('suggestions', () => {
  it('setSuggestions stores choices', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.setSuggestions(['opt1', 'opt2']);
    expect(s.suggestions).toEqual(['opt1', 'opt2']);
  });
});

describe('attachments', () => {
  it('addPendingAttachment gives auto-ID', () => {
    const s = useChatStore();
    s.addPendingAttachment({ name: 'f.txt', content: 'x' });
    expect(s.pendingAttachments[0].id).toBeDefined();
  });

  it('removePendingAttachment removes by id', () => {
    const s = useChatStore();
    s.addPendingAttachment({ name: 'a.txt', content: 'x' });
    const id = s.pendingAttachments[0].id;
    s.removePendingAttachment(id);
    expect(s.pendingAttachments).toHaveLength(0);
  });

  it('clearPendingAttachments empties the list', () => {
    const s = useChatStore();
    s.addPendingAttachment({ name: 'a.txt', content: 'x' });
    s.addPendingAttachment({ name: 'b.txt', content: 'y' });
    s.clearPendingAttachments();
    expect(s.pendingAttachments).toHaveLength(0);
  });
});

describe('saveableMessages', () => {
  it('only includes completed messages', () => {
    const s = useChatStore();
    s.addUserMessage('q', null, []);
    s.startAssistantMessage();
    expect(s.saveableMessages).toHaveLength(1);
  });

  it('includes chain on finalized assistant message', () => {
    const s = useChatStore();
    s.addUserMessage('q', null, []);
    s.startAssistantMessage();
    s.addThinking('hmm');
    s.addToolCall('search', {}, null);
    s.completeToolCall('search', 'result');
    s.finalizeAssistantMessage({ answer: 'done', sources: [], tool_calls_made: 1 });
    const saved = s.saveableMessages;
    const assistant = saved.find(m => m.role === 'assistant');
    expect(assistant.chain).toHaveLength(2);
  });

  it('includes provider on the assistant message when provider_used is set', () => {
    const s = useChatStore();
    s.addUserMessage('q', null, []);
    s.startAssistantMessage();
    s.finalizeAssistantMessage({
      answer: 'done', sources: [], tool_calls_made: 0,
      provider_used: { provider_id: 'gemini-1', kind: 'gemini', model: 'gemini-flash' },
    });
    const assistant = s.saveableMessages.find(m => m.role === 'assistant');
    expect(assistant.provider).toEqual({ provider_id: 'gemini-1', kind: 'gemini', model: 'gemini-flash' });
  });

  it('omits provider on the assistant message when provider_used is absent', () => {
    const s = useChatStore();
    s.addUserMessage('q', null, []);
    s.startAssistantMessage();
    s.finalizeAssistantMessage({ answer: 'done', sources: [], tool_calls_made: 0 });
    const assistant = s.saveableMessages.find(m => m.role === 'assistant');
    expect(assistant).not.toHaveProperty('provider');
  });
});

describe('loadFromHistory', () => {
  it('restores messages from saved format', () => {
    const s = useChatStore();
    s.loadFromHistory([
      { role: 'user', text: 'hi' },
      {
        role: 'assistant', text: 'hello', sources: [], tool_calls_made: 0,
        chain: [{ type: 'thinking', text: 'I thought' }],
      },
    ]);
    expect(msgs(s)).toHaveLength(2);
    expect(msgs(s)[1].chain[0].text).toBe('I thought');
  });

  it('backward compat: reconstructs chain from old thinking[] + tool_calls[]', () => {
    const s = useChatStore();
    s.loadFromHistory([{
      role: 'assistant', text: 'done', sources: [],
      thinking: ['old thought'],
      tool_calls: [{ name: 'search', input: {}, status: 'done', preview: 'r' }],
    }]);
    const chain = msgs(s)[0].chain;
    expect(chain.some(c => c.type === 'thinking')).toBe(true);
    expect(chain.some(c => c.type === 'tool_call')).toBe(true);
  });

  it('restores a pending question from history', () => {
    const s = useChatStore();
    s.loadFromHistory([{
      role: 'assistant', text: '', sources: [],
      question: { question: 'Which repo?', choices: ['a', 'b'] },
    }]);
    expect(msgs(s)[0].question).toEqual({ question: 'Which repo?', choices: ['a', 'b'] });
  });

  it('does not set question when absent from history', () => {
    const s = useChatStore();
    s.loadFromHistory([{ role: 'assistant', text: 'done', sources: [] }]);
    expect(msgs(s)[0].question).toBeUndefined();
  });

  it('restores a confirm_action chain entry from history with defaults for missing fields', () => {
    const s = useChatStore();
    s.loadFromHistory([{
      role: 'assistant', text: '', sources: [],
      chain: [{
        type: 'confirm_action',
        id: 'tc1',
        name: 'create_lxd_agent',
        input: { name: 'build-runner', flavor: 'small' },
        description: 'Create a small agent named build-runner',
      }],
    }]);
    expect(msgs(s)[0].chain).toEqual([{
      type: 'confirm_action',
      id: 'tc1',
      name: 'create_lxd_agent',
      input: { name: 'build-runner', flavor: 'small' },
      description: 'Create a small agent named build-runner',
      status: 'pending',
      steps: [],
      resultText: '',
    }]);
  });

  it('restores a resolved confirm_action chain entry preserving status/steps/result_text', () => {
    const s = useChatStore();
    s.loadFromHistory([{
      role: 'assistant', text: '', sources: [],
      chain: [{
        type: 'confirm_action',
        id: 'tc1',
        name: 'create_lxd_agent',
        input: { name: 'build-runner', flavor: 'small' },
        description: 'Create a small agent named build-runner',
        status: 'done',
        steps: [{ id: 'ensure_network', label: 'Ensuring network exists', status: 'done', detail: '' }],
        result_text: "Agent 'build-runner' created.",
      }],
    }]);
    expect(msgs(s)[0].chain).toEqual([{
      type: 'confirm_action',
      id: 'tc1',
      name: 'create_lxd_agent',
      input: { name: 'build-runner', flavor: 'small' },
      description: 'Create a small agent named build-runner',
      status: 'done',
      steps: [{ id: 'ensure_network', label: 'Ensuring network exists', status: 'done', detail: '' }],
      resultText: "Agent 'build-runner' created.",
    }]);
  });

  it('preserves the position of a confirm_action entry relative to surrounding tool calls', () => {
    const s = useChatStore();
    s.loadFromHistory([{
      role: 'assistant', text: '', sources: [],
      chain: [
        { type: 'tool_call', name: 'list_agents', input: {}, status: 'done', preview: '[]' },
        { type: 'confirm_action', id: 'tc1', name: 'delete_agent', input: {}, description: 'desc', status: 'done', steps: [], result_text: 'Agent deleted.' },
        { type: 'tool_call', name: 'run_command', input: {}, status: 'done', preview: 'ok' },
      ],
    }]);
    expect(msgs(s)[0].chain.map(c => c.type)).toEqual(['tool_call', 'confirm_action', 'tool_call']);
  });

  it('restores provider_used from a stored provider field', () => {
    const s = useChatStore();
    s.loadFromHistory([{
      role: 'assistant', text: 'done', sources: [],
      provider: { provider_id: 'anthropic-main', kind: 'anthropic', model: 'claude-sonnet-5' },
    }]);
    expect(msgs(s)[0].provider_used).toEqual({ provider_id: 'anthropic-main', kind: 'anthropic', model: 'claude-sonnet-5' });
  });

  it('provider_used is null when no provider field is stored', () => {
    const s = useChatStore();
    s.loadFromHistory([{ role: 'assistant', text: 'done', sources: [] }]);
    expect(msgs(s)[0].provider_used).toBeNull();
  });
});

describe('reset', () => {
  it('clears all state', () => {
    const s = useChatStore();
    s.addUserMessage('hi', null, []);
    s.startAssistantMessage();
    s.reset();
    expect(msgs(s)).toHaveLength(0);
    expect(s.loading).toBe(false);
  });
});

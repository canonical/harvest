import { describe, it, expect } from 'vitest';
import { useChatStore } from '../../src/stores/chat.js';

// ── helpers ───────────────────────────────────────────────────────────────────

function msgs(store) { return store.messages; }
function last(store) { return store.messages.at(-1); }

// ── initial state ─────────────────────────────────────────────────────────────

describe('initial state', () => {
  it('starts empty', () => {
    const s = useChatStore();
    expect(msgs(s)).toHaveLength(0);
    expect(s.loading).toBe(false);
    expect(s.pendingAttachments).toHaveLength(0);
  });
});

// ── addUserMessage ────────────────────────────────────────────────────────────

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

// ── assistant message lifecycle ───────────────────────────────────────────────

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

  it('setError marks error state', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.setError('something broke');
    expect(last(s).status).toBe('error');
    expect(last(s).error).toBe('something broke');
    expect(s.loading).toBe(false);
  });
});

// ── thinking chain ────────────────────────────────────────────────────────────

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
    // The streaming block was closed; a new non-streaming block was appended.
    expect(last(s).chain.at(-1).streaming).toBe(false);
    expect(last(s).chain.at(-1).text).toBe('full reasoning');
  });
});

// ── addThinkingDelta ─────────────────────────────────────────────────────────

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

// ── addTextDelta ──────────────────────────────────────────────────────────────

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

// ── tool calls ────────────────────────────────────────────────────────────────

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
    // pendingAnswer must be cleared and appear as a thinking block before the tool call
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

// ── question / choices ────────────────────────────────────────────────────────

describe('setQuestion', () => {
  it('attaches question to last assistant message', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.setQuestion('Pick one?', ['a', 'b']);
    expect(last(s).question).toEqual({ question: 'Pick one?', choices: ['a', 'b'] });
  });
});

// ── suggestions ──────────────────────────────────────────────────────────────

describe('suggestions', () => {
  it('setSuggestions stores choices', () => {
    const s = useChatStore();
    s.startAssistantMessage();
    s.setSuggestions(['opt1', 'opt2']);
    expect(s.suggestions).toEqual(['opt1', 'opt2']);
  });
});

// ── attachments ──────────────────────────────────────────────────────────────

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

// ── save & restore ────────────────────────────────────────────────────────────

describe('saveableMessages', () => {
  it('only includes completed messages', () => {
    const s = useChatStore();
    s.addUserMessage('q', null, []);
    s.startAssistantMessage();
    // still loading — not finalized
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
});

// ── reset ─────────────────────────────────────────────────────────────────────

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

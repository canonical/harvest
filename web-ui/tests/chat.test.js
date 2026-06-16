import { describe, it, expect } from 'vitest';
import {
  createChatState,
  addUserMessage,
  startAssistantMessage,
  addThinking,
  addToolCall,
  completeToolCall,
  updateToolCallDescription,
  finalizeAssistantMessage,
  setError,
  getMessages,
  isLoading,
  getSaveableMessages,
  loadFromHistory,
  addPendingAttachment,
  removePendingAttachment,
  getPendingAttachments,
  clearPendingAttachments,
} from '../src/chat.js';

// ── state creation ────────────────────────────────────────────────────────────

describe('createChatState', () => {
  it('starts with an empty message list', () => {
    const state = createChatState();
    expect(getMessages(state)).toHaveLength(0);
  });

  it('starts not loading', () => {
    const state = createChatState();
    expect(isLoading(state)).toBe(false);
  });
});

// ── addUserMessage ────────────────────────────────────────────────────────────

describe('addUserMessage', () => {
  it('appends a user message with the given text', () => {
    const s = createChatState();
    const s2 = addUserMessage(s, 'hello world');
    const msgs = getMessages(s2);
    expect(msgs).toHaveLength(1);
    expect(msgs[0].role).toBe('user');
    expect(msgs[0].text).toBe('hello world');
  });

  it('does not mutate the original state', () => {
    const s = createChatState();
    addUserMessage(s, 'hi');
    expect(getMessages(s)).toHaveLength(0);
  });
});

// ── startAssistantMessage ─────────────────────────────────────────────────────

describe('startAssistantMessage', () => {
  it('appends a pending assistant message', () => {
    const s = pipe(createChatState(), s => addUserMessage(s, 'q'));
    const s2 = startAssistantMessage(s);
    const msgs = getMessages(s2);
    expect(msgs[msgs.length - 1].role).toBe('assistant');
    expect(msgs[msgs.length - 1].status).toBe('loading');
  });

  it('sets the state to loading', () => {
    const s = startAssistantMessage(createChatState());
    expect(isLoading(s)).toBe(true);
  });

  it('has an empty tool_calls list initially', () => {
    const s = startAssistantMessage(createChatState());
    const msg = getMessages(s).at(-1);
    expect(msg.tool_calls).toEqual([]);
  });
});

// ── addToolCall ───────────────────────────────────────────────────────────────

describe('addToolCall', () => {
  it('appends a tool call to the latest assistant message', () => {
    const s = startAssistantMessage(createChatState());
    const s2 = addToolCall(s, { name: 'search_symbols', input: { query: 'foo' } });
    const msg = getMessages(s2).at(-1);
    expect(msg.tool_calls).toHaveLength(1);
    expect(msg.tool_calls[0].name).toBe('search_symbols');
    expect(msg.tool_calls[0].status).toBe('running');
  });

  it('can accumulate multiple tool calls', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addToolCall(s, { name: 'list_repositories', input: {} }),
      s => addToolCall(s, { name: 'search_symbols', input: {} }),
    );
    expect(getMessages(s).at(-1).tool_calls).toHaveLength(2);
  });
});

// ── completeToolCall ──────────────────────────────────────────────────────────

describe('completeToolCall', () => {
  it('marks the matching tool call as done', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addToolCall(s, { name: 'search_symbols', input: {} }),
      s => completeToolCall(s, { name: 'search_symbols', preview: 'result text' }),
    );
    const tc = getMessages(s).at(-1).tool_calls[0];
    expect(tc.status).toBe('done');
    expect(tc.preview).toBe('result text');
  });

  it('marks the last matching call when the same tool is called twice', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addToolCall(s, { name: 'search_symbols', input: { query: 'a' } }),
      s => completeToolCall(s, { name: 'search_symbols', preview: 'first result' }),
      s => addToolCall(s, { name: 'search_symbols', input: { query: 'b' } }),
      s => completeToolCall(s, { name: 'search_symbols', preview: 'second result' }),
    );
    const calls = getMessages(s).at(-1).tool_calls;
    expect(calls[0].status).toBe('done');
    expect(calls[0].preview).toBe('first result');
    expect(calls[1].status).toBe('done');
    expect(calls[1].preview).toBe('second result');
  });
});

// ── updateToolCallDescription ─────────────────────────────────────────────────

describe('updateToolCallDescription', () => {
  it('sets the description on the matching tool call by id', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addToolCall(s, { name: 'search_symbols', input: {} }),
    );
    const tc = getMessages(s).at(-1).tool_calls[0];
    const s2 = updateToolCallDescription(s, { id: tc.id, description: 'Searching for auth symbols' });
    expect(getMessages(s2).at(-1).tool_calls[0].description).toBe('Searching for auth symbols');
  });

  it('does not affect other tool calls', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addToolCall(s, { name: 'list_repositories', input: {} }),
      s => addToolCall(s, { name: 'search_symbols', input: {} }),
    );
    const calls = getMessages(s).at(-1).tool_calls;
    const s2 = updateToolCallDescription(s, { id: calls[1].id, description: 'Searching' });
    const updated = getMessages(s2).at(-1).tool_calls;
    expect(updated[0].description).toBeNull();
    expect(updated[1].description).toBe('Searching');
  });

  it('works on a finalized (done) assistant message', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addToolCall(s, { name: 'search_symbols', input: {} }),
      s => finalizeAssistantMessage(s, { answer: 'ok', sources: [], tool_calls_made: 1 }),
    );
    const tc = getMessages(s).at(-1).tool_calls[0];
    const s2 = updateToolCallDescription(s, { id: tc.id, description: 'Late arrival' });
    expect(getMessages(s2).at(-1).tool_calls[0].description).toBe('Late arrival');
  });

  it('each tool call gets a unique id', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addToolCall(s, { name: 'a', input: {} }),
      s => addToolCall(s, { name: 'b', input: {} }),
    );
    const [tc1, tc2] = getMessages(s).at(-1).tool_calls;
    expect(tc1.id).not.toBe(tc2.id);
  });

  it('is a no-op for an unknown id', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addToolCall(s, { name: 'search_symbols', input: {} }),
    );
    const s2 = updateToolCallDescription(s, { id: 999999, description: 'ghost' });
    expect(getMessages(s2).at(-1).tool_calls[0].description).toBeNull();
  });
});

// ── finalizeAssistantMessage ──────────────────────────────────────────────────

describe('finalizeAssistantMessage', () => {
  it('sets the answer and sources on the latest assistant message', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => finalizeAssistantMessage(s, {
        answer: 'The answer',
        sources: [{ repo: 'r', version: 'v1', file: 'f.rs', line: 1 }],
        tool_calls_made: 2,
      }),
    );
    const msg = getMessages(s).at(-1);
    expect(msg.answer).toBe('The answer');
    expect(msg.sources).toHaveLength(1);
    expect(msg.tool_calls_made).toBe(2);
    expect(msg.status).toBe('done');
  });

  it('sets loading to false', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => finalizeAssistantMessage(s, { answer: 'hi', sources: [], tool_calls_made: 0 }),
    );
    expect(isLoading(s)).toBe(false);
  });
});

// ── setError ──────────────────────────────────────────────────────────────────

describe('setError', () => {
  it('marks the latest assistant message with an error', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => setError(s, 'something went wrong'),
    );
    const msg = getMessages(s).at(-1);
    expect(msg.status).toBe('error');
    expect(msg.error).toBe('something went wrong');
  });

  it('sets loading to false on error', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => setError(s, 'oops'),
    );
    expect(isLoading(s)).toBe(false);
  });
});

// ── multi-turn conversation ───────────────────────────────────────────────────

describe('multi-turn conversation', () => {
  it('accumulates multiple user+assistant pairs', () => {
    const s = pipe(
      createChatState(),
      s => addUserMessage(s, 'first question'),
      s => startAssistantMessage(s),
      s => finalizeAssistantMessage(s, { answer: 'first answer', sources: [], tool_calls_made: 0 }),
      s => addUserMessage(s, 'second question'),
      s => startAssistantMessage(s),
      s => finalizeAssistantMessage(s, { answer: 'second answer', sources: [], tool_calls_made: 0 }),
    );
    const msgs = getMessages(s);
    expect(msgs).toHaveLength(4);
    expect(msgs[0].role).toBe('user');
    expect(msgs[1].role).toBe('assistant');
    expect(msgs[2].role).toBe('user');
    expect(msgs[3].role).toBe('assistant');
    expect(msgs[3].answer).toBe('second answer');
  });
});

// ── pending attachments ───────────────────────────────────────────────────────

const fakeImage = { id: 1, name: 'photo.png', mime_type: 'image/png', data: 'abc', preview_url: 'data:image/png;base64,abc' };
const fakePdf   = { id: 2, name: 'doc.pdf',   mime_type: 'application/pdf', data: 'pdf', preview_url: null };

describe('addPendingAttachment', () => {
  it('adds an attachment to the pending list', () => {
    const s = addPendingAttachment(createChatState(), fakeImage);
    expect(getPendingAttachments(s)).toHaveLength(1);
    expect(getPendingAttachments(s)[0].name).toBe('photo.png');
  });

  it('accumulates multiple attachments', () => {
    const s = pipe(
      createChatState(),
      s => addPendingAttachment(s, fakeImage),
      s => addPendingAttachment(s, fakePdf),
    );
    expect(getPendingAttachments(s)).toHaveLength(2);
  });

  it('does not mutate the original state', () => {
    const s = createChatState();
    addPendingAttachment(s, fakeImage);
    expect(getPendingAttachments(s)).toHaveLength(0);
  });
});

describe('removePendingAttachment', () => {
  it('removes an attachment by id', () => {
    const s = pipe(
      createChatState(),
      s => addPendingAttachment(s, fakeImage),
      s => addPendingAttachment(s, fakePdf),
      s => removePendingAttachment(s, 1),
    );
    expect(getPendingAttachments(s)).toHaveLength(1);
    expect(getPendingAttachments(s)[0].id).toBe(2);
  });

  it('is a no-op for an unknown id', () => {
    const s = addPendingAttachment(createChatState(), fakeImage);
    const s2 = removePendingAttachment(s, 999);
    expect(getPendingAttachments(s2)).toHaveLength(1);
  });
});

describe('clearPendingAttachments', () => {
  it('empties the pending list', () => {
    const s = pipe(
      createChatState(),
      s => addPendingAttachment(s, fakeImage),
      s => addPendingAttachment(s, fakePdf),
      s => clearPendingAttachments(s),
    );
    expect(getPendingAttachments(s)).toHaveLength(0);
  });
});

// ── attachments in messages ───────────────────────────────────────────────────

describe('addUserMessage with attachments', () => {
  it('stores attachments on the message', () => {
    const s = addUserMessage(createChatState(), 'hello', null, [fakeImage]);
    const msg = getMessages(s)[0];
    expect(msg.attachments).toHaveLength(1);
    expect(msg.attachments[0].name).toBe('photo.png');
  });

  it('defaults to empty attachments when not provided', () => {
    const s = addUserMessage(createChatState(), 'hello');
    expect(getMessages(s)[0].attachments).toEqual([]);
  });
});

describe('getSaveableMessages with tool calls', () => {
  it('includes tool_calls on saved assistant messages', () => {
    const s = pipe(
      createChatState(),
      s => addUserMessage(s, 'q'),
      s => startAssistantMessage(s),
      s => addToolCall(s, { name: 'search_symbols', input: { query: 'foo' } }),
      s => completeToolCall(s, { name: 'search_symbols', preview: 'result' }),
      s => finalizeAssistantMessage(s, { answer: 'done', sources: [], tool_calls_made: 1 }),
    );
    const saved = getSaveableMessages(s);
    const assistant = saved.find(m => m.role === 'assistant');
    expect(assistant.tool_calls).toHaveLength(1);
    expect(assistant.tool_calls[0].name).toBe('search_symbols');
    expect(assistant.tool_calls[0].preview).toBe('result');
    expect(assistant.tool_calls[0].status).toBe('done');
  });

  it('includes tool_calls_made on saved assistant messages', () => {
    const s = pipe(
      createChatState(),
      s => addUserMessage(s, 'q'),
      s => startAssistantMessage(s),
      s => addToolCall(s, { name: 'search_symbols', input: {} }),
      s => completeToolCall(s, { name: 'search_symbols', preview: '' }),
      s => finalizeAssistantMessage(s, { answer: 'done', sources: [], tool_calls_made: 1 }),
    );
    const saved = getSaveableMessages(s);
    expect(saved.find(m => m.role === 'assistant').tool_calls_made).toBe(1);
  });
});

describe('loadFromHistory with tool calls', () => {
  it('restores tool_calls on assistant messages', () => {
    const history = [
      { role: 'user', text: 'q' },
      {
        role: 'assistant', text: 'done', sources: [], tool_calls_made: 1,
        tool_calls: [{ name: 'search_symbols', input: { query: 'foo' }, preview: 'result', description: 'Searching foo', status: 'done' }],
      },
    ];
    const { messages } = loadFromHistory(history);
    const assistant = messages.find(m => m.role === 'assistant');
    expect(assistant.tool_calls).toHaveLength(1);
    expect(assistant.tool_calls[0].name).toBe('search_symbols');
    expect(assistant.tool_calls[0].status).toBe('done');
    expect(assistant.tool_calls[0].preview).toBe('result');
    expect(assistant.tool_calls[0].description).toBe('Searching foo');
  });

  it('restores tool_calls_made on assistant messages', () => {
    const history = [
      { role: 'user', text: 'q' },
      { role: 'assistant', text: 'done', sources: [], tool_calls_made: 3, tool_calls: [] },
    ];
    const { messages } = loadFromHistory(history);
    expect(messages.find(m => m.role === 'assistant').tool_calls_made).toBe(3);
  });

  it('defaults to empty tool_calls when absent in history', () => {
    const history = [{ role: 'assistant', text: 'hi', sources: [] }];
    const { messages } = loadFromHistory(history);
    expect(messages[0].tool_calls).toEqual([]);
  });
});

describe('getSaveableMessages with attachments', () => {
  it('includes attachments in saved user messages', () => {
    const s = pipe(
      createChatState(),
      s => addUserMessage(s, 'see this', null, [fakeImage]),
      s => startAssistantMessage(s),
      s => finalizeAssistantMessage(s, { answer: 'ok', sources: [], tool_calls_made: 0 }),
    );
    const saved = getSaveableMessages(s);
    expect(saved[0].attachments).toHaveLength(1);
    expect(saved[0].attachments[0].name).toBe('photo.png');
  });

  it('saves an empty attachments array when message has none', () => {
    const s = pipe(
      createChatState(),
      s => addUserMessage(s, 'no attachments'),
      s => startAssistantMessage(s),
      s => finalizeAssistantMessage(s, { answer: 'ok', sources: [], tool_calls_made: 0 }),
    );
    const saved = getSaveableMessages(s);
    expect(saved[0].attachments).toEqual([]);
  });
});

describe('loadFromHistory with attachments', () => {
  it('restores attachments on user messages', () => {
    const history = [
      { role: 'user', text: 'see this', attachments: [{ name: 'img.png', mime_type: 'image/png', data: 'xyz', preview_url: null }] },
      { role: 'assistant', text: 'got it', sources: [] },
    ];
    const { messages } = loadFromHistory(history);
    expect(messages[0].attachments).toHaveLength(1);
    expect(messages[0].attachments[0].name).toBe('img.png');
  });

  it('defaults to empty attachments when history message has none', () => {
    const history = [{ role: 'user', text: 'hello' }];
    const { messages } = loadFromHistory(history);
    expect(messages[0].attachments).toEqual([]);
  });
});

// ── addThinking ───────────────────────────────────────────────────────────────

describe('addThinking', () => {
  it('adds a thinking item to the chain', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addThinking(s, { text: 'Let me think...' }),
    );
    const chain = getMessages(s).at(-1).chain;
    expect(chain).toHaveLength(1);
    expect(chain[0]).toEqual({ type: 'thinking', text: 'Let me think...' });
  });

  it('can accumulate multiple thinking items', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addThinking(s, { text: 'first thought' }),
      s => addThinking(s, { text: 'second thought' }),
    );
    const chain = getMessages(s).at(-1).chain;
    expect(chain).toHaveLength(2);
    expect(chain[0].text).toBe('first thought');
    expect(chain[1].text).toBe('second thought');
  });

  it('does not affect tool_calls', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addThinking(s, { text: 'hmm' }),
    );
    expect(getMessages(s).at(-1).tool_calls).toHaveLength(0);
  });
});

// ── chain ordering ────────────────────────────────────────────────────────────

describe('chain ordering', () => {
  it('starts with an empty chain', () => {
    const s = startAssistantMessage(createChatState());
    expect(getMessages(s).at(-1).chain).toEqual([]);
  });

  it('preserves thinking → tool_call order', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addThinking(s, { text: 'I should search first' }),
      s => addToolCall(s, { name: 'search_symbols', input: {} }),
    );
    const chain = getMessages(s).at(-1).chain;
    expect(chain).toHaveLength(2);
    expect(chain[0].type).toBe('thinking');
    expect(chain[1].type).toBe('tool_call');
  });

  it('interleaves thinking and tool calls across multiple rounds', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addThinking(s, { text: 'step 1' }),
      s => addToolCall(s, { name: 'tool_a', input: {} }),
      s => addThinking(s, { text: 'step 2' }),
      s => addToolCall(s, { name: 'tool_b', input: {} }),
    );
    const chain = getMessages(s).at(-1).chain;
    expect(chain.map(c => c.type)).toEqual(['thinking', 'tool_call', 'thinking', 'tool_call']);
    expect(chain[0].text).toBe('step 1');
    expect(chain[1].name).toBe('tool_a');
    expect(chain[2].text).toBe('step 2');
    expect(chain[3].name).toBe('tool_b');
  });

  it('tool_calls array mirrors tool_call items in chain order', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addThinking(s, { text: 'hmm' }),
      s => addToolCall(s, { name: 'alpha', input: {} }),
      s => addThinking(s, { text: 'ok' }),
      s => addToolCall(s, { name: 'beta', input: {} }),
    );
    const msg = getMessages(s).at(-1);
    expect(msg.tool_calls.map(tc => tc.name)).toEqual(['alpha', 'beta']);
  });

  it('completeToolCall updates the matching item in chain', () => {
    const s = pipe(
      startAssistantMessage(createChatState()),
      s => addThinking(s, { text: 'thinking' }),
      s => addToolCall(s, { name: 'search', input: {} }),
      s => completeToolCall(s, { name: 'search', preview: 'found it' }),
    );
    const chain = getMessages(s).at(-1).chain;
    const tc = chain.find(c => c.type === 'tool_call');
    expect(tc.status).toBe('done');
    expect(tc.preview).toBe('found it');
  });
});

// ── getSaveableMessages with chain ────────────────────────────────────────────

describe('getSaveableMessages with chain', () => {
  it('saves chain on the assistant message', () => {
    const s = pipe(
      createChatState(),
      s => addUserMessage(s, 'q'),
      s => startAssistantMessage(s),
      s => addThinking(s, { text: 'thinking…' }),
      s => addToolCall(s, { name: 'search', input: {} }),
      s => completeToolCall(s, { name: 'search', preview: 'result' }),
      s => finalizeAssistantMessage(s, { answer: 'done', sources: [], tool_calls_made: 1 }),
    );
    const saved = getSaveableMessages(s);
    const assistant = saved.find(m => m.role === 'assistant');
    expect(assistant.chain).toBeDefined();
    expect(assistant.chain).toHaveLength(2);
    expect(assistant.chain[0]).toEqual({ type: 'thinking', text: 'thinking…' });
    expect(assistant.chain[1].type).toBe('tool_call');
    expect(assistant.chain[1].name).toBe('search');
  });
});

// ── loadFromHistory with chain ────────────────────────────────────────────────

describe('loadFromHistory with chain', () => {
  it('restores chain from saved format', () => {
    const history = [{
      role: 'assistant', text: 'done', sources: [], tool_calls_made: 1,
      chain: [
        { type: 'thinking', text: 'I should search' },
        { type: 'tool_call', id: 1, name: 'search', input: {}, status: 'done', preview: 'result', description: null },
      ],
      tool_calls: [{ name: 'search', input: {}, status: 'done', preview: 'result', description: null }],
    }];
    const { messages } = loadFromHistory(history);
    const msg = messages[0];
    expect(msg.chain).toHaveLength(2);
    expect(msg.chain[0].type).toBe('thinking');
    expect(msg.chain[0].text).toBe('I should search');
    expect(msg.chain[1].type).toBe('tool_call');
  });

  it('reconstructs chain from old format (separate thinking + tool_calls)', () => {
    const history = [{
      role: 'assistant', text: 'done', sources: [], tool_calls_made: 1,
      thinking: ['I should search'],
      tool_calls: [{ name: 'search', input: {}, status: 'done', preview: 'result', description: null }],
    }];
    const { messages } = loadFromHistory(history);
    const msg = messages[0];
    expect(msg.chain).toBeDefined();
    const thinkingItems = msg.chain.filter(c => c.type === 'thinking');
    const toolCallItems = msg.chain.filter(c => c.type === 'tool_call');
    expect(thinkingItems).toHaveLength(1);
    expect(toolCallItems).toHaveLength(1);
  });

  it('produces empty chain when neither chain nor thinking/tool_calls present', () => {
    const history = [{ role: 'assistant', text: 'hi', sources: [] }];
    const { messages } = loadFromHistory(history);
    expect(messages[0].chain).toEqual([]);
  });
});

// ── helper ────────────────────────────────────────────────────────────────────

function pipe(initial, ...fns) {
  return fns.reduce((v, f) => f(v), initial);
}

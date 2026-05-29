import { describe, it, expect } from 'vitest';
import {
  createChatState,
  addUserMessage,
  startAssistantMessage,
  addToolCall,
  completeToolCall,
  finalizeAssistantMessage,
  setError,
  getMessages,
  isLoading,
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

// ── helper ────────────────────────────────────────────────────────────────────

function pipe(initial, ...fns) {
  return fns.reduce((v, f) => f(v), initial);
}

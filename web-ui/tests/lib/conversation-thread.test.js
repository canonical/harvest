import { describe, it, expect } from 'vitest';
import { createConversationThreadState, INTERRUPTED_MESSAGE } from '../../src/lib/conversation-thread.js';

describe('loadFromHistory', () => {
  it('shows a saved interrupted answer as interrupted, keeping its partial text', () => {
    const thread = createConversationThreadState();
    thread.loadFromHistory([
      { role: 'user', text: 'q' },
      { role: 'assistant', text: 'half an ans', status: 'interrupted', chain: [] },
    ]);
    const answer = thread.messages.value[1];
    expect(answer.status).toBe('error');
    expect(answer.error).toBe(INTERRUPTED_MESSAGE);
    expect(answer.answer).toBe('half an ans');
  });

  it('leaves completed answers done', () => {
    const thread = createConversationThreadState();
    thread.loadFromHistory([{ role: 'assistant', text: 'all', chain: [] }]);
    expect(thread.messages.value[0].status).toBe('done');
  });
});

describe('prependHistory', () => {
  it('puts earlier messages before the loaded ones', () => {
    const thread = createConversationThreadState();
    thread.loadFromHistory([{ role: 'user', text: 'latest' }]);
    thread.prependHistory([{ role: 'user', text: 'first' }, { role: 'assistant', text: 'reply', chain: [] }]);
    expect(thread.messages.value.map(m => m.text ?? m.answer)).toEqual(['first', 'reply', 'latest']);
  });
});

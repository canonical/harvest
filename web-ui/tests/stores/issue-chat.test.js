import { describe, it, expect } from 'vitest';
import { useIssueChatStore } from '../../src/stores/issue-chat.js';

describe('useIssueChatStore', () => {
  it('starts empty for a fresh issue id', () => {
    const s = useIssueChatStore('issue-1');
    expect(s.messages).toHaveLength(0);
    expect(s.loading).toBe(false);
  });

  it('returns the same store instance for repeated calls with the same issue id', () => {
    const a = useIssueChatStore('issue-2');
    a.addUserMessage('hello', null, []);
    const b = useIssueChatStore('issue-2');
    expect(b.messages).toHaveLength(1);
    expect(b.messages[0].text).toBe('hello');
  });

  it('keeps state isolated between different issue ids', () => {
    const a = useIssueChatStore('issue-3');
    const b = useIssueChatStore('issue-4');
    a.addUserMessage('for issue 3', null, []);
    expect(a.messages).toHaveLength(1);
    expect(b.messages).toHaveLength(0);
  });
});

import { describe, it, expect } from 'vitest';
import { terminalMenuItems } from '../../src/lib/session-menus.js';

describe('terminalMenuItems', () => {
  const terminals = [{ id: 't1', title: 'Terminal 1' }, { id: 't2', title: 'server' }];

  it('lists the session terminals, offers a new one and ends the current one', () => {
    const items = terminalMenuItems(terminals, 't2');
    expect(items.map((item) => item.id)).toEqual(['t1', 't2', 'terminals-divider', 'new-terminal', 'end-terminal']);
    expect(items[1].checked).toBe(true);
    expect(items.at(-1)).toMatchObject({ action: 'end-terminal', danger: true });
  });

  it('offers only a new terminal when there is none yet', () => {
    expect(terminalMenuItems([], null).map((item) => item.id)).toEqual(['new-terminal']);
  });
});

import { describe, it, expect } from 'vitest';
import { diffFiles, diffSummary } from '../../src/lib/diff-lines.js';
import { splitFailure } from '../../src/lib/preview-failure.js';
import { portProblem, previewSettings } from '../../src/lib/preview-settings.js';
import { chatMenuItems, previewMenuItems } from '../../src/lib/session-menus.js';
import { paneShortcut } from '../../src/lib/pane-shortcuts.js';
import { tabActivity } from '../../src/composables/useChatInstance.js';

const DIFF = [
  'diff --git a/src/a.js b/src/a.js',
  'index 1..2 100644',
  '--- a/src/a.js',
  '+++ b/src/a.js',
  '@@ -1,2 +1,2 @@',
  '-old',
  '+new',
  '+more',
  'diff --git a/README.md b/README.md',
  'new file mode 100644',
  '@@ -0,0 +1 @@',
  '+hello',
].join('\n');

describe('diffs', () => {
  it('are split per file with their counts and without git metadata', () => {
    const files = diffFiles(DIFF);
    expect(files.map(f => [f.path, f.added, f.removed])).toEqual([['src/a.js', 2, 1], ['README.md', 1, 0]]);
    expect(files[0].lines.map(l => l.text)).toEqual(['@@ -1,2 +1,2 @@', '-old', '+new', '+more']);
  });

  it('are summarised in one line', () => {
    expect(diffSummary(['c1'], diffFiles(DIFF))).toBe('1 commit · 2 files · +3 −1');
    expect(diffSummary([], [])).toBe('0 commits · 0 files · +0 −0');
  });
});

describe('preview failures', () => {
  it('separate the advice from the output', () => {
    const { summary, details } = splitFailure('error: `npm` is not installed.\n\nLast output:\nbash: npm: not found');
    expect(summary).toBe('`npm` is not installed.');
    expect(details).toBe('Last output:\nbash: npm: not found');
    expect(splitFailure('one line').details).toBe('');
  });
});

describe('preview settings', () => {
  it('check ports before anything is sent', () => {
    expect(portProblem(0, 'web')).toMatch('between 1 and 65535');
    expect(portProblem(3000.5, 'api')).toMatch('between 1 and 65535');
    expect(portProblem(5173, 'web', [{ port: 5173 }])).toBe('Port 5173 is already used by another preview of this session.');
    expect(portProblem(null, 'cli')).toBe('');
    expect(portProblem(8000, 'web', [{ port: 5173 }])).toBe('');
  });

  it('drop the port and command of command-line previews', () => {
    expect(previewSettings({ kind: 'cli', port: 3000, startCommand: 'x' })).toEqual({ kind: 'cli', port: null, start_command: null });
    expect(previewSettings({ kind: 'web', port: 3000, startCommand: '  ' })).toEqual({ kind: 'web', port: 3000, start_command: null });
  });
});

describe('session menus', () => {
  const chats = [{ id: 'c1', title: 'Fix login' }, { id: 'c2', title: 'Add search' }];

  it('list chats with the current one checked and its actions last', () => {
    const items = chatMenuItems(chats, 'c1');
    expect(items[0].id).toBe('new-chat');
    expect(items.find(i => i.id === 'c1').checked).toBe(true);
    expect(items.slice(-2).map(i => i.action)).toEqual(['rename-chat', 'delete-chat']);
  });

  it('filter chats by title', () => {
    const titles = chatMenuItems(chats, 'c1', 'search').filter(i => i.checked !== undefined).map(i => i.label);
    expect(titles).toEqual(['Add search']);
    expect(chatMenuItems(chats, 'c1', 'zzz').some(i => i.label === 'No chats match')).toBe(true);
  });

  it('list previews with their origin and expiry', () => {
    const now = Date.parse('2026-01-01T00:00:00Z');
    const items = previewMenuItems([{ id: 'p1', kind: 'web', port: 5173, chat_title: 'Fix login', expires_at: '2026-01-01T03:00:00Z' }], 'p1', now);
    expect(items[0]).toMatchObject({ label: 'Preview · web :5173', hint: 'from “Fix login” · expires in 3 h', checked: true });
    expect(items.at(-1).action).toBe('new-preview');
  });
});

describe('pane shortcuts', () => {
  const key = (code, extra = {}) => ({ code, ctrlKey: true, altKey: true, shiftKey: false, metaKey: false, ...extra });

  it('map Ctrl+Alt combinations to pane actions', () => {
    expect(paneShortcut(key('KeyT'))).toBe('tab');
    expect(paneShortcut(key('Backslash'))).toBe('right');
    expect(paneShortcut(key('Minus'))).toBe('bottom');
    expect(paneShortcut(key('KeyW'))).toBe('close');
  });

  it('ignore other keys and modifiers', () => {
    expect(paneShortcut(key('KeyT', { altKey: false }))).toBeNull();
    expect(paneShortcut(key('KeyT', { shiftKey: true }))).toBeNull();
    expect(paneShortcut(key('KeyX'))).toBeNull();
  });
});

describe('tab activity', () => {
  const instance = (loading, replies) => ({ loading, messages: Array.from({ length: replies }, () => ({ role: 'assistant', status: 'done' })) });

  it('shows work in progress and unseen replies in background tabs', () => {
    expect(tabActivity(instance(true, 1), 1, false)).toBe('working');
    expect(tabActivity(instance(false, 2), 1, false)).toBe('unseen');
    expect(tabActivity(instance(false, 2), 1, true)).toBeNull();
    expect(tabActivity(instance(false, 2), undefined, false)).toBeNull();
    expect(tabActivity(null, 0, false)).toBeNull();
  });
});

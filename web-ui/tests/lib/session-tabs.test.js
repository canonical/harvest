import { describe, it, expect } from 'vitest';
import {
  SESSION_TAB_KINDS, chatTab, changesTab, previewTab, previewLabel, initialTree, tabTitle, expiresIn, emptySessionTab,
} from '../../src/lib/session-tabs.js';

describe('session tabs', () => {
  it('offers chat, changes and preview tabs in that order', () => {
    expect(SESSION_TAB_KINDS.map(k => k.kind)).toEqual(['chat', 'changes', 'preview']);
  });

  it('opens a new session on its first chat', () => {
    const tree = initialTree([{ id: 'c1', title: 'Fix login' }]);
    expect(tree.root.type).toBe('leaf');
    expect(tree.root.tabs).toHaveLength(1);
    expect(tree.root.tabs[0]).toMatchObject({ kind: 'chat', conversationId: 'c1' });
    expect(tree.root.activeTabId).toBe(tree.root.tabs[0].tabId);
  });

  it('builds tabs of each kind with their own ids', () => {
    expect(chatTab({ id: 'c2', title: 'x' })).toMatchObject({ kind: 'chat', conversationId: 'c2' });
    expect(changesTab()).toMatchObject({ kind: 'changes' });
    expect(previewTab({ id: 'p1', kind: 'web', port: 5173 })).toMatchObject({ kind: 'preview', previewId: 'p1' });
    expect(previewTab(null)).toMatchObject({ kind: 'preview', previewId: null });
    expect(chatTab().tabId).not.toBe(chatTab().tabId);
    expect(emptySessionTab()).toEqual({ kind: 'chat', conversationId: null, title: 'Chat' });
  });

  it('names previews by kind and port', () => {
    expect(previewLabel({ kind: 'web', port: 5173 })).toBe('Preview · web :5173');
    expect(previewLabel({ kind: 'cli', port: null })).toBe('Preview · cli');
    expect(previewLabel(null)).toBe('Preview');
  });

  it('titles tabs from the current chats and previews', () => {
    const context = { chats: [{ id: 'c1', title: 'Renamed' }], previews: [{ id: 'p1', kind: 'api', port: 8000 }] };
    expect(tabTitle({ kind: 'chat', conversationId: 'c1', title: 'Old' }, context)).toBe('Renamed');
    expect(tabTitle({ kind: 'chat', conversationId: 'gone' }, context)).toBe('Chat');
    expect(tabTitle({ kind: 'preview', previewId: 'p1' }, context)).toBe('Preview · api :8000');
    expect(tabTitle({ kind: 'preview', previewId: 'gone' }, context)).toBe('Preview');
    expect(tabTitle({ kind: 'changes' }, context)).toBe('Changes');
  });

  it('tells how long a preview has left', () => {
    const now = Date.parse('2026-01-01T00:00:00Z');
    expect(expiresIn({ expires_at: '2026-01-01T05:30:00Z' }, now)).toBe('expires in 5 h');
    expect(expiresIn({ expires_at: '2026-01-01T00:20:00Z' }, now)).toBe('expires in 20 min');
    expect(expiresIn({ expires_at: '2025-12-31T23:00:00Z' }, now)).toBe('expiring');
  });
});

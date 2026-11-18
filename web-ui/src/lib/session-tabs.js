import { createTab, createTree } from './pane-layout.js';

export const SESSION_TAB_KINDS = Object.freeze([
  { kind: 'chat', label: 'Chat', icon: 'chat' },
  { kind: 'changes', label: 'Changes', icon: 'changes' },
  { kind: 'preview', label: 'Preview', icon: 'preview' },
  { kind: 'jobs', label: 'Jobs', icon: 'jobs' },
  { kind: 'terminal', label: 'Terminal', icon: 'terminal' },
]);

function chatFields(chat) {
  return { kind: 'chat', conversationId: chat?.id ?? null, title: chat?.title ?? 'Chat' };
}

export function chatTab(chat) {
  return createTab(chatFields(chat));
}

export function changesTab() {
  return createTab({ kind: 'changes', title: 'Changes' });
}

export function jobsTab() {
  return createTab({ kind: 'jobs', title: 'Jobs' });
}

export function terminalTab(terminal) {
  return createTab({ kind: 'terminal', terminalId: terminal?.id ?? null, title: terminal?.title ?? 'Terminal' });
}

export function terminalLabel(terminal, liveTitle = '') {
  const name = terminal?.title ?? 'Terminal';
  return liveTitle ? `${name} · ${liveTitle}` : name;
}

export function previewTab(preview) {
  return createTab({ kind: 'preview', previewId: preview?.id ?? null, title: previewLabel(preview) });
}

export function emptySessionTab() {
  return { kind: 'chat', conversationId: null, title: 'Chat' };
}

export function previewLabel(preview) {
  if (!preview) return 'Preview';
  const port = preview.port ? ` :${preview.port}` : '';
  return `Preview · ${preview.kind}${port}`;
}

export function initialTree(chats) {
  return createTree(chatFields(chats[0]));
}

export function tabTitle(tab, { chats = [], previews = [], terminals = [], liveTitles = new Map() } = {}) {
  if (tab.kind === 'terminal') return terminalLabel(terminals.find(t => t.id === tab.terminalId), liveTitles.get(tab.tabId));
  if (tab.kind === 'changes') return 'Changes';
  if (tab.kind === 'jobs') return 'Jobs';
  if (tab.kind === 'preview') return previewLabel(previews.find(p => p.id === tab.previewId));
  return chats.find(c => c.id === tab.conversationId)?.title ?? 'Chat';
}

export function expiresIn(preview, now = Date.now()) {
  const left = new Date(preview.expires_at).getTime() - now;
  if (!Number.isFinite(left) || left <= 0) return 'expiring';
  const hours = Math.floor(left / 3_600_000);
  if (hours >= 1) return `expires in ${hours} h`;
  return `expires in ${Math.max(1, Math.ceil(left / 60_000))} min`;
}

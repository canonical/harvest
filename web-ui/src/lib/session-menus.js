import { expiresIn, previewLabel } from './session-tabs.js';

export function chatMenuItems(chats, currentId, filter = '') {
  const needle = filter.trim().toLowerCase();
  const matching = chats.filter(c => !needle || c.title.toLowerCase().includes(needle));
  const items = [{ id: 'new-chat', label: 'New chat', icon: 'chat' }, { id: 'chats-divider', divider: true }];
  if (matching.length) {
    items.push(...matching.map(c => ({ id: c.id, label: c.title, checked: c.id === currentId })));
  } else {
    items.push({ id: 'no-chats', heading: true, label: needle ? 'No chats match' : 'No chats yet' });
  }
  if (chats.some(c => c.id === currentId)) {
    items.push(
      { id: 'actions-divider', divider: true },
      { id: 'rename-chat', label: 'Rename this chat…', action: 'rename-chat' },
      { id: 'delete-chat', label: 'Delete this chat', action: 'delete-chat', danger: true },
    );
  }
  return items;
}

export function terminalMenuItems(terminals, currentId) {
  const items = terminals.map(t => ({ id: t.id, label: t.title, checked: t.id === currentId }));
  if (items.length) items.push({ id: 'terminals-divider', divider: true });
  items.push({ id: 'new-terminal', label: 'New terminal', icon: 'terminal' });
  if (terminals.some(t => t.id === currentId)) {
    items.push({ id: 'end-terminal', label: 'End this terminal', action: 'end-terminal', danger: true });
  }
  return items;
}

export function previewMenuItems(previews, currentId, now = Date.now()) {
  const items = previews.map(p => ({
    id: p.id,
    label: previewLabel(p),
    hint: [p.chat_title && `from “${p.chat_title}”`, expiresIn(p, now)].filter(Boolean).join(' · '),
    checked: p.id === currentId,
  }));
  if (items.length) items.push({ id: 'previews-divider', divider: true });
  items.push({ id: 'new-preview', label: 'New preview…', icon: 'preview', action: 'new-preview' });
  return items;
}

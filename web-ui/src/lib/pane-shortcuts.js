const ACTIONS = {
  KeyT: 'tab',
  Backslash: 'right',
  Minus: 'bottom',
  KeyW: 'close',
};

export function paneShortcut(event) {
  if (!event.ctrlKey || !event.altKey || event.shiftKey || event.metaKey) return null;
  return ACTIONS[event.code] ?? null;
}

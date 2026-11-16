const FOCUSABLE = [
  'a[href]', 'button:not([disabled])', 'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])', 'textarea:not([disabled])', 'iframe', '[tabindex]:not([tabindex="-1"])',
].join(',');

export function focusables(container) {
  return [...(container?.querySelectorAll(FOCUSABLE) ?? [])].filter(el => !el.closest('[hidden]'));
}

export function keepFocusInside(event, container) {
  if (event.key !== 'Tab') return;
  const items = focusables(container);
  if (!items.length) {
    event.preventDefault();
    return;
  }
  const first = items[0];
  const last = items[items.length - 1];
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
  }
}

export function moveFocus(items, current, step) {
  if (!items.length) return null;
  const index = items.indexOf(current);
  const next = index < 0 ? (step > 0 ? 0 : items.length - 1) : (index + step + items.length) % items.length;
  items[next].focus();
  return items[next];
}

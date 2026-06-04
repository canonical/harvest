const BASE = '/conversations';

// ── API ───────────────────────────────────────────────────────────────────────

async function apiFetch(url, options = {}) {
  const res = await fetch(url, {
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `Request failed (${res.status})`);
  }
  return res.json();
}

export const listConversations   = ()            => apiFetch(BASE);
export const createConversation  = (title)       => apiFetch(BASE, { method: 'POST', body: JSON.stringify({ title }) });
export const getConversation     = (id)          => apiFetch(`${BASE}/${id}`);
export const updateConversation  = (id, payload) => apiFetch(`${BASE}/${id}`, { method: 'PUT', body: JSON.stringify(payload) });
export const deleteConversation  = (id)          => apiFetch(`${BASE}/${id}`, { method: 'DELETE' });

// ── Date grouping ─────────────────────────────────────────────────────────────

function dayStart(date) {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

function groupConversations(conversations) {
  const now       = Date.now();
  const todayMs   = dayStart(new Date(now));
  const yestMs    = todayMs - 86_400_000;
  const weekMs    = todayMs - 7 * 86_400_000;

  const groups = [
    { label: 'Today',      items: [] },
    { label: 'Yesterday',  items: [] },
    { label: 'Last 7 days', items: [] },
    { label: 'Older',      items: [] },
  ];

  for (const c of conversations) {
    const ms = dayStart(new Date(c.updated_at));
    if      (ms >= todayMs) groups[0].items.push(c);
    else if (ms >= yestMs)  groups[1].items.push(c);
    else if (ms >= weekMs)  groups[2].items.push(c);
    else                    groups[3].items.push(c);
  }

  return groups.filter(g => g.items.length > 0);
}

// ── Render sidebar ────────────────────────────────────────────────────────────

function esc(s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;').replace(/</g, '&lt;')
    .replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

const trashIcon = `
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"
    stroke="currentColor" stroke-width="2" stroke-linecap="round"
    stroke-linejoin="round" width="16" height="16" aria-hidden="true">
    <polyline points="3 6 5 6 21 6"/>
    <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/>
    <path d="M10 11v6M14 11v6"/>
    <path d="M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/>
  </svg>`;

export function renderConvList(container, conversations, activeId, {
  onSelect, onDelete,
  showCreator = false,
  canDelete   = () => true,
}) {
  if (!conversations.length) {
    container.innerHTML = '<p class="conv-empty">No past conversations yet.</p>';
    return;
  }

  const groups = groupConversations(conversations);

  container.innerHTML = groups.map(g => `
    <div class="conv-group">
      <div class="conv-group-label">${esc(g.label)}</div>
      ${g.items.map(c => `
        <div class="conv-item${c.id === activeId ? ' conv-item--active' : ''}"
          data-conv-id="${esc(c.id)}" role="button" tabindex="0"
          aria-label="${esc(c.title)}" aria-pressed="${c.id === activeId}">
          <div class="conv-item__body">
            <span class="conv-item__title">${esc(c.title)}</span>
            ${showCreator && c.created_by_name
              ? `<span class="conv-item__creator">${esc(c.created_by_name)}</span>`
              : ''}
          </div>
          ${canDelete(c) ? `
          <button class="conv-item__delete" data-delete-conv="${esc(c.id)}"
            aria-label="Delete conversation" title="Delete" tabindex="-1" type="button">
            ${trashIcon}
          </button>` : ''}
        </div>
      `).join('')}
    </div>
  `).join('');

  container.querySelectorAll('.conv-item').forEach(el => {
    const id = el.dataset.convId;
    el.addEventListener('click', e => {
      if (e.target.closest('.conv-item__delete')) return;
      onSelect(id);
    });
    el.addEventListener('keydown', e => {
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onSelect(id); }
    });
  });

  container.querySelectorAll('[data-delete-conv]').forEach(btn => {
    btn.addEventListener('click', e => {
      e.stopPropagation();
      onDelete(btn.dataset.deleteConv);
    });
  });
}

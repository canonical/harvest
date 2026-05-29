/**
 * Render a parsed JSON value as a Vanilla-Framework-styled HTML string.
 * Arrays of objects become full tables with headers.
 * Plain objects become two-column key/value tables.
 * Primitives are returned as escaped text (numbers/booleans in <code>).
 *
 * @param {unknown} value
 * @returns {string} HTML string
 */
export function renderJsonToHtml(value) {
  if (value === null || value === undefined) return '<em>—</em>';
  if (typeof value === 'string') return esc(value);
  if (typeof value === 'number' || typeof value === 'boolean') return `<code>${esc(String(value))}</code>`;
  if (Array.isArray(value)) return renderArray(value);
  if (typeof value === 'object') return renderObject(value);
  return esc(String(value));
}

/**
 * Try to parse a preview string as JSON and render it; fall back to escaped
 * plain text when the string is truncated or not JSON.
 *
 * @param {string|null} text
 * @returns {string} HTML string
 */
export function renderPreviewToHtml(text) {
  if (!text) return '';
  try {
    return renderJsonToHtml(JSON.parse(text));
  } catch {
    return `<span class="tool-data__fallback">${esc(text)}</span>`;
  }
}

// ── internal ──────────────────────────────────────────────────────────────────

function renderObject(obj) {
  const entries = Object.entries(obj);
  if (entries.length === 0) return '<em>—</em>';
  const rows = entries
    .map(([k, v]) => `<tr><th scope="row">${esc(k)}</th><td>${leaf(v)}</td></tr>`)
    .join('');
  return `<table class="p-table tool-data__table"><tbody>${rows}</tbody></table>`;
}

function renderArray(arr) {
  if (arr.length === 0) return '<em>—</em>';
  if (arr.every(isPlainObject)) return renderObjectArray(arr);
  return `<ul class="p-list tool-data__list">${arr.map(v => `<li>${leaf(v)}</li>`).join('')}</ul>`;
}

function renderObjectArray(arr) {
  const keys = [...new Set(arr.flatMap(obj => Object.keys(obj)))];
  const head = keys.map(k => `<th>${esc(k)}</th>`).join('');
  const rows = arr
    .map(obj => `<tr>${keys.map(k => `<td>${leaf(obj[k] ?? null)}</td>`).join('')}</tr>`)
    .join('');
  return `<table class="p-table tool-data__table">
    <thead><tr>${head}</tr></thead>
    <tbody>${rows}</tbody>
  </table>`;
}

/** Render a leaf value (no further recursion into objects). */
function leaf(v) {
  if (v === null || v === undefined) return '<em>—</em>';
  if (typeof v === 'object') return `<code>${esc(JSON.stringify(v))}</code>`;
  if (typeof v === 'number' || typeof v === 'boolean') return `<code>${esc(String(v))}</code>`;
  return esc(String(v));
}

function isPlainObject(v) {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function esc(str) {
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

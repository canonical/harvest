import hljs from 'highlight.js';

const EXT_TO_LANG = {
  rs: 'rust', py: 'python', js: 'javascript', ts: 'typescript',
  go: 'go', java: 'java', cpp: 'cpp', cc: 'cpp', c: 'c',
  rb: 'ruby', sh: 'bash', toml: 'toml', yaml: 'yaml', yml: 'yaml',
};

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
 * Try to parse a preview string as JSON and render it nicely.
 * If the result is an array of objects that all carry a `source` field,
 * renders syntax-highlighted code instead of a table.
 * If the string is a truncated JSON array, recover the complete objects
 * that appear before the cut and render them with a truncation notice.
 * Falls back to escaped plain text for anything else.
 *
 * @param {string|null} text
 * @param {string|null} [filePath] — source file path, used to infer language
 * @returns {string} HTML string
 */
export function renderPreviewToHtml(text, filePath = null) {
  if (!text) return '';

  // 1. Try a full parse first.
  try {
    const parsed = JSON.parse(text);
    if (Array.isArray(parsed) && parsed.length === 0) {
      return '<span class="tool-data__empty">No results returned</span>';
    }
    const sourceHtml = tryRenderAsSource(parsed, filePath);
    if (sourceHtml !== null) return sourceHtml;
    return renderJsonToHtml(parsed);
  } catch { /* fall through */ }

  // 2. Try to recover complete objects from a truncated JSON array.
  const partial = tryParsePartialArray(text);
  if (partial) {
    const sourceHtml = tryRenderAsSource(partial, filePath);
    if (sourceHtml !== null) {
      return sourceHtml + '<span class="tool-data__truncated">… preview truncated</span>';
    }
    return renderJsonToHtml(partial)
      + '<span class="tool-data__truncated">… preview truncated</span>';
  }

  // 3. Plain-text fallback.
  return `<span class="tool-data__fallback">${esc(text)}</span>`;
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
  const s = String(v);
  // Shorten long absolute paths — keep the last 4 components, full path as tooltip.
  const shortened = shortenAbsPath(s);
  if (shortened) {
    return `<span class="tool-data__path" title="${esc(s)}">${esc(shortened)}</span>`;
  }
  return esc(s);
}

/**
 * If `str` is an absolute path with more than 4 components, return a
 * shortened version showing only the last 4 components. Otherwise null.
 */
function shortenAbsPath(str) {
  if (!str.startsWith('/')) return null;
  const parts = str.split('/').filter(Boolean);
  if (parts.length <= 4) return null;
  return parts.slice(-4).join('/');
}

/**
 * If `parsed` is an array of objects where every item has a `source` string,
 * render each item as a syntax-highlighted code block with a compact meta
 * header (version · name · line range). Returns null otherwise.
 */
function tryRenderAsSource(parsed, filePath) {
  if (!Array.isArray(parsed) || parsed.length === 0) return null;
  if (!parsed.every(item => isPlainObject(item) && typeof item.source === 'string')) return null;

  const lang = langFromPath(filePath);
  const blocks = parsed.map(item => {
    const parts = [];
    if (item.version) parts.push(item.version);
    if (item.name) parts.push(item.name);
    if (item.start_line != null && item.end_line != null) {
      parts.push(`L${item.start_line}–${item.end_line}`);
    }
    const meta = parts.length > 0
      ? `<div class="tool-source__meta">${esc(parts.join(' · '))}</div>`
      : '';
    const highlighted = hljs.highlight(item.source, { language: lang, ignoreIllegals: true }).value;
    return `${meta}<pre class="tool-source__pre"><code class="hljs language-${esc(lang)}">${highlighted}</code></pre>`;
  });
  return `<div class="tool-source">${blocks.join('')}</div>`;
}

function langFromPath(filePath) {
  if (!filePath) return 'plaintext';
  const ext = filePath.split('.').pop().toLowerCase();
  const lang = EXT_TO_LANG[ext] ?? ext;
  return hljs.getLanguage(lang) ? lang : 'plaintext';
}

/**
 * Recover the complete array elements from a truncated JSON array string.
 * Walks the string character-by-character tracking depth and string state,
 * identifies the end of each top-level array element, then tries to parse
 * the prefix up to the last complete element.
 * Returns the parsed array or null if recovery is not possible.
 */
function tryParsePartialArray(text) {
  const t = text.trimStart();
  if (!t.startsWith('[')) return null;

  let depth = 0;
  let inStr = false;
  let esc_ = false;
  let lastTopLevelClose = -1;

  for (let i = 0; i < t.length; i++) {
    const c = t[i];
    if (esc_) { esc_ = false; continue; }
    if (inStr) {
      if (c === '\\') esc_ = true;
      else if (c === '"') inStr = false;
      continue;
    }
    if (c === '"') { inStr = true; continue; }
    if (c === '{' || c === '[') depth++;
    else if (c === '}' || c === ']') {
      depth--;
      // depth === 1 means we just closed a top-level array element
      if (depth === 1) lastTopLevelClose = i;
    }
  }

  if (lastTopLevelClose < 0) return null;

  try {
    return JSON.parse(t.slice(0, lastTopLevelClose + 1) + ']');
  } catch {
    return null;
  }
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

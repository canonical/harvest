import { renderMarkdown } from './markdown.js';
import { fetchProjectOverview } from './api.js';

// ── Per-project generation state ──────────────────────────────────────────────
//
// _gens holds state for every project that is currently being generated,
// even when the user has switched away.  Only the project matching _displayPid
// draws to the DOM; all others keep accumulating step data silently.

const _gens = new Map();
// projectId → { steps: [], startMs: number, timerHandle: number|null }

let _displayPid = null;   // project whose progress is shown right now

// ── HTML renderer ─────────────────────────────────────────────────────────────

function _renderHtml(container, html) {
  const safe = html
    .replace(/<script\b[^<]*(?:(?!<\/script>)<[^<]*)*<\/script>/gi, '')
    .replace(/\bon\w+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]*)/gi, '');

  const styles = [...safe.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/gi)]
    .map(m => m[1]).join('\n');
  const markup = safe.replace(/<style[^>]*>[\s\S]*?<\/style>/gi, '');

  let styleEl = document.getElementById('ov-generated-style');
  if (!styleEl) {
    styleEl = document.createElement('style');
    styleEl.id = 'ov-generated-style';
    document.head.appendChild(styleEl);
  }
  styleEl.textContent = styles;
  container.innerHTML = markup;
}

// ── Progress icons ────────────────────────────────────────────────────────────

const ICON_CHAT = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>`;
const ICON_GRID = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="3" width="7" height="7"/><rect x="14" y="3" width="7" height="7"/><rect x="14" y="14" width="7" height="7"/><rect x="3" y="14" width="7" height="7"/></svg>`;

function _iconDone() {
  return `<svg viewBox="0 0 16 16" fill="currentColor" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
    <path fill-rule="evenodd" d="M8 15A7 7 0 1 0 8 1a7 7 0 0 0 0 14zm3.857-9.809a.75.75 0 0 0-1.214-.882l-3.483 4.79-1.88-1.88a.75.75 0 1 0-1.06 1.061l2.5 2.5a.75.75 0 0 0 1.137-.089l4-5.5z" clip-rule="evenodd"/>
  </svg>`;
}

function _iconSpinner() {
  return `<svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
    <circle cx="8" cy="8" r="6.5" stroke="currentColor" stroke-opacity="0.2" stroke-width="1.5"/>
    <path d="M8 1.5 A6.5 6.5 0 0 1 14.5 8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
  </svg>`;
}

// ── Generation state helpers ──────────────────────────────────────────────────

function _startGen(pid) {
  _gens.set(pid, { steps: [], startMs: Date.now(), timerHandle: null });
}

function _cleanupGen(pid) {
  const gen = _gens.get(pid);
  if (!gen) return;
  clearInterval(gen.timerHandle);
  _gens.delete(pid);
}

function _startTimer(pid) {
  const gen = _gens.get(pid);
  if (!gen || gen.timerHandle) return;
  gen.timerHandle = setInterval(() => {
    if (_displayPid === pid) _drawProgress();
  }, 120);
}

function _stopTimer(pid) {
  const gen = _gens.get(pid);
  if (!gen) return;
  clearInterval(gen.timerHandle);
  gen.timerHandle = null;
}

function _onStepEvent(ev, pid) {
  const gen = _gens.get(pid);
  if (!gen) return;

  if (ev.kind === 'stage') {
    const prev = gen.steps.find(s => s.state === 'running');
    if (prev) { prev.state = 'done'; prev.endMs = Date.now(); }
    gen.steps.push({ label: ev.action, state: 'running', startMs: Date.now(), endMs: null, tools: [] });
  } else if (ev.kind === 'tool') {
    const cur = [...gen.steps].reverse().find(s => s.state === 'running') ?? gen.steps.at(-1);
    if (cur) cur.tools.push(ev.action);
  }

  if (_displayPid === pid) _drawProgress();
}

function _markAllDone(pid) {
  const gen = _gens.get(pid);
  if (!gen) return;
  const running = gen.steps.find(s => s.state === 'running');
  if (running) { running.state = 'done'; running.endMs = Date.now(); }
  if (_displayPid === pid) _drawProgress();
}

// ── Progress panel renderer ───────────────────────────────────────────────────

function _drawProgress() {
  const el = document.getElementById('overview-progress');
  if (!el || el.hidden) return;
  const gen = _gens.get(_displayPid);
  if (!gen) return;

  const totalSec = ((Date.now() - gen.startMs) / 1000).toFixed(1);

  const stepsHtml = gen.steps.map(step => {
    const done = step.state === 'done';
    const dur = done
      ? `${((step.endMs - step.startMs) / 1000).toFixed(1)}s`
      : `${((Date.now() - step.startMs) / 1000).toFixed(1)}s`;

    const toolsHtml = step.tools.length
      ? `<ul class="ovprog__tools">${step.tools.map(t =>
          `<li class="ovprog__tool"><span class="ovprog__tool-bullet" aria-hidden="true">↳</span>${_esc(t)}</li>`
        ).join('')}</ul>`
      : '';

    return `<li class="ovprog__step ovprog__step--${step.state}">
      <span class="ovprog__step-icon${done ? '' : ' ovprog__step-icon--spin'}">${done ? _iconDone() : _iconSpinner()}</span>
      <span class="ovprog__step-body">
        <span class="ovprog__step-label">${_esc(step.label)}</span>
        ${toolsHtml}
      </span>
      <span class="ovprog__step-dur">${dur}</span>
    </li>`;
  }).join('');

  el.innerHTML = `<div class="ovprog">
    <div class="ovprog__header">
      <span class="ovprog__header-spinner">${_iconSpinner()}</span>
      <span class="ovprog__header-label">Generating overview</span>
      <span class="ovprog__header-clock">${totalSec}s</span>
    </div>
    <ol class="ovprog__list" aria-label="Generation progress">${stepsHtml}</ol>
  </div>`;
}

// Show the progress panel for the currently displayed project.
function _showProgressPanel() {
  _hide('overview-content',    true);
  _hide('overview-empty',      true);
  _hide('overview-loading',    true);
  _hide('overview-no-project', true);
  _hide('overview-progress',   false);
  _drawProgress();
}

// ── SSE stream reader (shared by POST /regenerate and GET /events) ────────────

async function _readStream(pid, fetchPromise) {
  try {
    const resp = await fetchPromise;
    if (!resp.ok) return;

    const reader  = resp.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';

    outer: while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const blocks = buffer.split('\n\n');
      buffer = blocks.pop() ?? '';
      for (const block of blocks) {
        for (const line of block.split('\n')) {
          if (!line.startsWith('data: ')) continue;
          try {
            const ev = JSON.parse(line.slice(6));
            if (ev.type === 'overview_step') _onStepEvent(ev, pid);
            else if (ev.type === 'overview_done') break outer;
          } catch {}
        }
      }
    }
  } catch (e) {
    console.error('Overview stream error:', e);
  }
}

// ── Regeneration ──────────────────────────────────────────────────────────────

async function _runRegenerate(pid) {
  if (_gens.has(pid)) return;   // already generating this project

  _startGen(pid);
  if (_displayPid === pid) {
    _showProgressPanel();
    _setSpinning(true);
    _setStatusText('Generating…');
  }
  _startTimer(pid);

  await _readStream(pid, fetch(
    `/projects/${encodeURIComponent(pid)}/overview/regenerate`,
    { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' },
  ));

  await _finishGen(pid);
}

// Called when a page-refresh or cross-tab reconnect detects an in-progress generation.
async function _subscribeToEvents(pid) {
  if (_gens.has(pid)) return;   // already tracking locally

  _startGen(pid);
  if (_displayPid === pid) {
    _showProgressPanel();
    _setSpinning(true);
    _setStatusText('Generating…');
  }
  _startTimer(pid);

  await _readStream(pid, fetch(
    `/projects/${encodeURIComponent(pid)}/overview/events`,
  ));

  await _finishGen(pid);
}

async function _finishGen(pid) {
  _markAllDone(pid);
  if (_displayPid === pid) _drawProgress();

  // Let the user see the completed timeline briefly.
  await _sleep(600);

  _stopTimer(pid);
  _cleanupGen(pid);

  if (_displayPid === pid) {
    _hide('overview-progress', true);
    try {
      const data = await fetchProjectOverview(pid);
      renderOverview(data);
    } catch { /* show whatever we have */ }
    _setStatusText('');
    _setSpinning(false);
  }
}

// ── Page lifecycle ────────────────────────────────────────────────────────────

export function initOverviewPage() {
  const btn = document.getElementById('overview-regenerate-btn');
  if (!btn) return;
  btn.addEventListener('click', () => {
    const pid = btn.dataset.projectId;
    if (pid) _runRegenerate(pid);
  });
}

export function showOverviewNoProject() {
  _displayPid = null;
  _hide('overview-no-project', false);
  _hide('overview-content',    true);
  _hide('overview-empty',      true);
  _hide('overview-loading',    true);
  _hide('overview-progress',   true);
  const btn = document.getElementById('overview-regenerate-btn');
  if (btn) btn.hidden = true;
  _setUpdated('');
  _setStatusText('');
}

export async function onOverviewPageShow(pid) {
  _displayPid = pid;

  const btn = document.getElementById('overview-regenerate-btn');
  if (btn) { btn.dataset.projectId = pid; btn.hidden = false; }

  // Case 1: generation already tracked in this JS session — resume display.
  if (_gens.has(pid)) {
    _showProgressPanel();
    _startTimer(pid);          // restart the ticker (was paused on page hide)
    _setSpinning(true);
    _setStatusText('Generating…');
    return;
  }

  // Case 2: no local state — ask the server.
  _hide('overview-loading',    false);
  _hide('overview-content',    true);
  _hide('overview-empty',      true);
  _hide('overview-progress',   true);

  try {
    const data = await fetchProjectOverview(pid);

    if (data.generating) {
      // Case 2a: server says generation is in progress — subscribe.
      _hide('overview-loading', true);
      _subscribeToEvents(pid);  // intentionally not awaited; runs in background
    } else {
      // Case 2b: nothing running, show current overview (or empty state).
      _hide('overview-loading', true);
      renderOverview(data);
    }
  } catch {
    _hide('overview-loading', true);
    showOverviewNoProject();
  }
}

export function onOverviewPageHide() {
  // Pause the display timer but leave gen state intact.
  if (_displayPid) _stopTimer(_displayPid);
  _displayPid = null;
  document.getElementById('ov-generated-style')?.remove();
}

export function renderOverview(data) {
  _hide('overview-no-project', true);
  _hide('overview-loading',    true);
  _hide('overview-progress',   true);

  if (!data || !data.current_status) {
    _hide('overview-content', true);
    _hide('overview-empty',   false);
    _setUpdated('');
    const emptyEl = document.getElementById('overview-empty');
    if (emptyEl) {
      emptyEl.innerHTML = data && !data.has_conversations
        ? `<div class="overview-splash__icon">${ICON_CHAT}</div>
           <p class="overview-splash__title">No overview yet</p>
           <p class="overview-splash__text">Start chatting in the <strong>Chat</strong> tab and an overview will be generated from your conversations.</p>`
        : `<div class="overview-splash__icon">${ICON_GRID}</div>
           <p class="overview-splash__title">Ready to generate</p>
           <p class="overview-splash__text">Click the refresh button to generate an overview of this project's environment.</p>`;
    }
    return;
  }

  _hide('overview-empty',   true);
  _hide('overview-content', false);

  const bodyEl = document.getElementById('overview-body');
  if (bodyEl) {
    const isHtml = data.current_status.trimStart().startsWith('<');
    if (isHtml) {
      _renderHtml(bodyEl, data.current_status);
    } else {
      bodyEl.innerHTML = renderMarkdown(data.current_status);
      _layoutTables(bodyEl);
    }
  }
  _setUpdated(formatLastUpdated(data.current_status_updated_at));
}

export function formatLastUpdated(isoString) {
  if (!isoString) return '';
  try { return `Last updated ${new Date(isoString).toLocaleString()}`; }
  catch { return ''; }
}

// ── Markdown table layout (legacy fallback) ───────────────────────────────────

function _layoutTables(container) {
  const nodes = [...container.childNodes];
  if (!nodes.some((n, i) => n.nodeName === 'H3' && nodes[i + 1]?.nodeName === 'TABLE')) return;
  const grid = document.createElement('div');
  grid.className = 'overview-tables-grid';
  let i = 0;
  while (i < nodes.length) {
    const node = nodes[i];
    if (node.nodeName === 'H3' && nodes[i + 1]?.nodeName === 'TABLE') {
      const block = document.createElement('div');
      block.className = 'overview-table-block';
      block.appendChild(node);
      block.appendChild(nodes[i + 1]);
      grid.appendChild(block);
      i += 2;
    } else {
      grid.appendChild(node);
      i++;
    }
  }
  container.appendChild(grid);
}

// ── DOM helpers ───────────────────────────────────────────────────────────────

function _hide(id, hidden) {
  const el = document.getElementById(id);
  if (el) el.hidden = hidden;
}

function _setUpdated(text) {
  const el = document.getElementById('overview-last-updated');
  if (el) el.textContent = text;
}

function _setStatusText(text) {
  const el = document.getElementById('overview-status-text');
  if (!el) return;
  el.hidden = !text;
  el.textContent = text;
}

function _setSpinning(on) {
  const btn = document.getElementById('overview-regenerate-btn');
  if (!btn) return;
  btn.disabled = on;
  btn.classList.toggle('overview-refresh-btn--spinning', on);
}

function _sleep(ms) { return new Promise(r => setTimeout(r, ms)); }

function _esc(s) {
  return String(s)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;')
    .replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

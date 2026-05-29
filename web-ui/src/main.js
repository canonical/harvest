import './style.css';
import { applyStoredTheme, nextTheme, getTheme, getThemeIcon, getThemeLabel } from './theme.js';
import { queryStream, fetchToolDescription, fetchRepositories } from './api.js';
import { renderMarkdown, buildFileUrl, formatCitation } from './markdown.js';
import { renderJsonToHtml, renderPreviewToHtml } from './format.js';
import {
  createChatState,
  addUserMessage,
  startAssistantMessage,
  addToolCall,
  completeToolCall,
  updateToolCallDescription,
  finalizeAssistantMessage,
  setError,
  getMessages,
  isLoading,
} from './chat.js';

// ── Helpers ───────────────────────────────────────────────────────────────────

function escapeHtml(str) {
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

// ── State ─────────────────────────────────────────────────────────────────────

let state = createChatState();
/** @type {Object.<string, string>} repo name → clone URL */
let repoUrlMap = {};

// ── DOM refs ──────────────────────────────────────────────────────────────────

const messagesEl   = document.getElementById('messages');
const inputEl      = document.getElementById('query-input');
const sendBtn      = document.getElementById('send-btn');
const statusDot    = document.getElementById('status-dot');
const themeBtnEl   = document.getElementById('theme-btn');

// ── Render ────────────────────────────────────────────────────────────────────

function render() {
  // Snapshot expand/open state before replacing the DOM
  const prevAssistantEls = [...messagesEl.querySelectorAll('.message--assistant')];
  const expandedWraps = new Set(
    prevAssistantEls.flatMap((el, i) =>
      el.querySelector('.tool-calls-wrap.expanded') ? [i] : []
    )
  );
  const openToolCalls = new Set(
    prevAssistantEls.flatMap((el, i) =>
      [...el.querySelectorAll('.tool-call.open')].map((_, j) => `${i}:${j}`)
    )
  );

  const messages = getMessages(state);
  messagesEl.innerHTML = messages.map(renderMessage).join('');

  const loading = isLoading(state);
  sendBtn.disabled = loading;
  inputEl.disabled = loading;

  // Scroll to bottom
  messagesEl.scrollTop = messagesEl.scrollHeight;

  // Restore expand/open state
  [...messagesEl.querySelectorAll('.message--assistant')].forEach((el, i) => {
    if (expandedWraps.has(i)) {
      const wrap = el.querySelector('.tool-calls-wrap');
      if (wrap) {
        wrap.classList.add('expanded');
        const chevron = wrap.querySelector('.tool-calls-toggle__chevron');
        if (chevron) chevron.textContent = '▴';
        const btn = wrap.querySelector('.tool-calls-toggle');
        if (btn) btn.setAttribute('aria-expanded', 'true');
      }
    }
    el.querySelectorAll('.tool-call').forEach((tc, j) => {
      if (openToolCalls.has(`${i}:${j}`)) tc.classList.add('open');
    });
  });

  // Attach tool call toggle listeners
  messagesEl.querySelectorAll('.tool-call__header').forEach(header => {
    header.addEventListener('click', () => {
      header.closest('.tool-call').classList.toggle('open');
    });
  });

  // Attach tool-calls section expand/collapse listeners
  messagesEl.querySelectorAll('.tool-calls-toggle').forEach(btn => {
    btn.addEventListener('click', () => {
      const wrap = btn.closest('.tool-calls-wrap');
      const expanded = wrap.classList.toggle('expanded');
      btn.setAttribute('aria-expanded', String(expanded));
      btn.querySelector('.tool-calls-toggle__chevron').textContent = expanded ? '▴' : '▾';
    });
  });
}

function renderMessage(msg) {
  if (msg.role === 'user') {
    return `
      <div class="message message--user">
        <div class="message__bubble">${escapeHtml(msg.text)}</div>
      </div>
    `;
  }

  if (msg.status === 'loading' && msg.tool_calls.length === 0) {
    return `
      <div class="message message--assistant">
        <div class="message__bubble">
          <div class="loading-dots"><span></span><span></span><span></span></div>
        </div>
      </div>
    `;
  }

  if (msg.status === 'error') {
    return `
      <div class="message message--assistant message--error">
        <div class="message__bubble">
          <strong>Error:</strong> ${escapeHtml(msg.error)}
        </div>
      </div>
    `;
  }

  // Assistant message (loading with tool calls or fully done)
  const n = msg.tool_calls.length;
  const toolCallsHtml = n > 0 ? `
    <div class="tool-calls-wrap">
      <div class="tool-calls-clip">
        <div class="tool-calls">
          ${msg.tool_calls.map(renderToolCall).join('')}
        </div>
      </div>
      <button class="tool-calls-toggle" type="button" aria-expanded="false">
        <span class="tool-calls-toggle__chevron">▾</span> ${n} tool call${n === 1 ? '' : 's'}
      </button>
    </div>
  ` : '';

  const bodyHtml = msg.answer ? `
    <div class="message__body">${renderMarkdown(msg.answer, repoUrlMap)}</div>
  ` : (msg.status === 'loading' ? `
    <div class="loading-dots"><span></span><span></span><span></span></div>
  ` : '');

  const sourcesHtml = (msg.sources && msg.sources.length > 0) ? `
    <div class="sources">
      <div class="sources__title">Sources (${msg.sources.length})</div>
      <div class="source-chips">
        ${msg.sources.map(s => {
          const repoUrl = repoUrlMap[s.repo];
          const fileUrl = repoUrl ? buildFileUrl(repoUrl, s.version, s.file, s.line) : null;
          const title = escapeHtml(`${s.repo}:${s.version}:${s.file}:${s.line}`);
          const label = escapeHtml(formatCitation(s));
          return fileUrl
            ? `<a href="${escapeHtml(fileUrl)}" class="source-chip" target="_blank" rel="noopener noreferrer" title="${title}">${label}</a>`
            : `<span class="source-chip" title="${title}">${label}</span>`;
        }).join('')}
      </div>
    </div>
  ` : '';

  return `
    <div class="message message--assistant">
      <div class="message__bubble">
        ${toolCallsHtml}
        ${bodyHtml}
        ${sourcesHtml}
      </div>
    </div>
  `;
}

function renderToolCall(tc) {
  const statusLabel = tc.status === 'running' ? 'Running…' : 'Done';
  const label = tc.description
    ? escapeHtml(tc.description)
    : `<span class="tool-call__name--bare">${escapeHtml(tc.name)}</span>`;
  const previewHtml = tc.preview ? `
    <div class="tool-call__label">Result preview</div>
    <div class="tool-data">${renderPreviewToHtml(tc.preview, tc.input?.file ?? null)}</div>
  ` : '';

  return `
    <div class="tool-call ${tc.status}">
      <div class="tool-call__header">
        <div class="tool-call__icon"></div>
        <span class="tool-call__name">${label}</span>
        <span class="tool-call__status">${statusLabel}</span>
      </div>
      <div class="tool-call__body">
        <div class="tool-call__label">Tool: ${escapeHtml(tc.name)}</div>
        <div class="tool-call__label">Input</div>
        <div class="tool-data">${renderJsonToHtml(tc.input)}</div>
        ${previewHtml}
      </div>
    </div>
  `;
}

// ── Send ──────────────────────────────────────────────────────────────────────

async function sendQuery() {
  const query = inputEl.value.trim();
  if (!query || isLoading(state)) return;

  inputEl.value = '';
  autoResize();

  state = addUserMessage(state, query);
  state = startAssistantMessage(state);
  render();

  try {
    await queryStream(query, (event) => {
      if (event.type === 'tool_call') {
        state = addToolCall(state, { name: event.name, input: event.input });
        const tc = getMessages(state).at(-1).tool_calls.at(-1);
        fetchToolDescription(event.name, event.input).then((description) => {
          if (description) {
            state = updateToolCallDescription(state, { id: tc.id, description });
            render();
          }
        });
      } else if (event.type === 'tool_result') {
        state = completeToolCall(state, { name: event.name, preview: event.preview });
      } else if (event.type === 'done') {
        state = finalizeAssistantMessage(state, {
          answer: event.answer,
          sources: event.sources,
          tool_calls_made: event.tool_calls_made,
        });
      } else if (event.type === 'error') {
        state = setError(state, event.message);
      }
      render();
    });
  } catch (err) {
    state = setError(state, err.message);
    render();
  }
}

// ── Input auto-resize ─────────────────────────────────────────────────────────

function autoResize() {
  inputEl.style.height = 'auto';
  inputEl.style.height = `${Math.min(inputEl.scrollHeight, 128)}px`;
}

// ── Health check ──────────────────────────────────────────────────────────────

async function checkHealth() {
  try {
    const resp = await fetch('/health');
    if (resp.ok) {
      statusDot.className = 'status-dot connected';
      statusDot.title = 'Server connected';
    } else {
      throw new Error('not ok');
    }
  } catch {
    statusDot.className = 'status-dot error';
    statusDot.title = 'Server unreachable';
  }
}

// ── Event listeners ───────────────────────────────────────────────────────────

sendBtn.addEventListener('click', sendQuery);

inputEl.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    sendQuery();
  }
});

inputEl.addEventListener('input', autoResize);

// ── Theme ─────────────────────────────────────────────────────────────────────

function updateThemeButton() {
  const t = getTheme();
  themeBtnEl.innerHTML = getThemeIcon(t);
  themeBtnEl.title = getThemeLabel(t);
  themeBtnEl.setAttribute('aria-label', `Switch theme (current: ${getThemeLabel(t)})`);
}

themeBtnEl.addEventListener('click', () => {
  nextTheme();
  updateThemeButton();
});

// ── Bootstrap ─────────────────────────────────────────────────────────────────

applyStoredTheme();
updateThemeButton();
checkHealth();
fetchRepositories().then((repos) => {
  repoUrlMap = Object.fromEntries(
    repos.filter(r => r.url).map(r => [r.name, r.url])
  );
});
render();

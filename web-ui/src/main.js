import './style.css';
import { queryStream } from './api.js';
import { fetchRepositories } from './api.js';
import { renderMarkdown, formatCitation } from './markdown.js';
import {
  createChatState,
  addUserMessage,
  startAssistantMessage,
  addToolCall,
  completeToolCall,
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

// ── DOM refs ──────────────────────────────────────────────────────────────────

const messagesEl   = document.getElementById('messages');
const inputEl      = document.getElementById('query-input');
const sendBtn      = document.getElementById('send-btn');
const statusDot    = document.getElementById('status-dot');
const repoListEl   = document.getElementById('repo-list');

// ── Repository sidebar ────────────────────────────────────────────────────────

async function loadRepositories() {
  const repos = await fetchRepositories();
  if (repos.length === 0) {
    repoListEl.innerHTML = '<li class="repo-name" style="color:#aaa">None yet</li>';
    return;
  }
  repoListEl.innerHTML = repos.map(r => `
    <li>
      <div class="repo-name">${escapeHtml(r.name)}</div>
      <div class="repo-versions">${r.versions.slice(0, 5).map(v => escapeHtml(v)).join(', ')}${r.versions.length > 5 ? '…' : ''}</div>
    </li>
  `).join('');
}

// ── Render ────────────────────────────────────────────────────────────────────

function render() {
  const messages = getMessages(state);
  messagesEl.innerHTML = messages.map(renderMessage).join('');

  const loading = isLoading(state);
  sendBtn.disabled = loading;
  inputEl.disabled = loading;

  // Scroll to bottom
  messagesEl.scrollTop = messagesEl.scrollHeight;

  // Attach tool call toggle listeners
  messagesEl.querySelectorAll('.tool-call__header').forEach(header => {
    header.addEventListener('click', () => {
      header.closest('.tool-call').classList.toggle('open');
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
  const toolCallsHtml = msg.tool_calls.length > 0 ? `
    <div class="tool-calls">
      ${msg.tool_calls.map(renderToolCall).join('')}
    </div>
  ` : '';

  const bodyHtml = msg.answer ? `
    <div class="message__body">${renderMarkdown(msg.answer)}</div>
  ` : (msg.status === 'loading' ? `
    <div class="loading-dots"><span></span><span></span><span></span></div>
  ` : '');

  const sourcesHtml = (msg.sources && msg.sources.length > 0) ? `
    <div class="sources">
      <div class="sources__title">Sources (${msg.sources.length})</div>
      <div class="source-chips">
        ${msg.sources.map(s => `<span class="source-chip" title="${escapeHtml(`${s.repo}:${s.version}:${s.file}:${s.line}`)}">${escapeHtml(formatCitation(s))}</span>`).join('')}
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
  const inputJson = JSON.stringify(tc.input, null, 2);
  const previewHtml = tc.preview ? `
    <div class="tool-call__label">Result preview</div>
    <pre class="tool-call__preview">${escapeHtml(tc.preview)}</pre>
  ` : '';

  return `
    <div class="tool-call ${tc.status}">
      <div class="tool-call__header">
        <div class="tool-call__icon"></div>
        <span class="tool-call__name">${escapeHtml(tc.name)}</span>
        <span class="tool-call__status">${statusLabel}</span>
      </div>
      <div class="tool-call__body">
        <div class="tool-call__label">Input</div>
        <pre class="tool-call__input">${escapeHtml(inputJson)}</pre>
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

// ── Bootstrap ─────────────────────────────────────────────────────────────────

checkHealth();
loadRepositories();
render();

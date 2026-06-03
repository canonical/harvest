import './vanilla.scss';
import './style.css';
import { applyStoredTheme, nextTheme, getTheme, getThemeIcon, getThemeLabel } from './theme.js';
import { queryStream, fetchToolDescription, fetchRepositories, setUnauthorizedHandler } from './api.js';
import { initRepositoriesPage, onRepositoriesPageShow, onRepositoriesPageHide } from './repositories.js';
import { initDocumentationPage } from './documentation.js';
import { renderMarkdown, buildFileUrl } from './markdown.js';
import { renderJsonToHtml, renderPreviewToHtml } from './format.js';
import { escapeHtml as esc, addCopyButtons } from './utils.js';
import { initSourcePanel, closeSourcePanel } from './source-panel.js';
import { mountInlineGraphs } from './inline-graph.js';
import {
  fetchMe, logout, initAuthPages, showLoginPage, hideAuthPages, isAdmin, setUser,
} from './auth.js';
import { initAdminPage } from './admin.js';
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
  getSaveableMessages,
  loadFromHistory,
} from './chat.js';
import {
  listConversations,
  createConversation,
  getConversation,
  updateConversation,
  deleteConversation,
  renderConvList,
} from './conversations.js';

let state = createChatState();
let repoUrlMap = {};
let activeConvId = null;
let conversations = [];

const messagesEl  = document.getElementById('messages');
const inputEl     = document.getElementById('query-input');
const sendBtn     = document.getElementById('send-btn');
const themeBtnEl  = document.getElementById('theme-btn');
const navEl       = document.getElementById('app-sidebar');
const navToggleEl = document.getElementById('nav-toggle');
const navCloseEl  = document.getElementById('nav-close');

function render() {
  const prevAssistantEls = [...messagesEl.querySelectorAll('.message--assistant')];
  const openGroups = new Set(
    prevAssistantEls.flatMap((el, i) =>
      el.querySelector('details.tc-group[open]') ? [i] : []
    )
  );
  const openSteps = new Set(
    prevAssistantEls.flatMap((el, i) =>
      [...el.querySelectorAll('.tc-step--open')].map(s => `${i}:${s.dataset.tcIdx}`)
    )
  );

  const messages = getMessages(state);
  messagesEl.innerHTML = messages.map(renderMessage).join('');

  const loading = isLoading(state);
  sendBtn.disabled = loading;
  inputEl.disabled = loading;

  const atBottom = messagesEl.scrollHeight - messagesEl.scrollTop - messagesEl.clientHeight < 80;
  if (atBottom) messagesEl.scrollTop = messagesEl.scrollHeight;

  [...messagesEl.querySelectorAll('.message--assistant')].forEach((el, i) => {
    const group = el.querySelector('details.tc-group');
    if (group && openGroups.has(i)) group.open = true;
    el.querySelectorAll('.tc-step').forEach(step => {
      if (openSteps.has(`${i}:${step.dataset.tcIdx}`)) {
        step.classList.add('tc-step--open');
        const detail = step.querySelector('.tc-step__detail');
        if (detail) detail.hidden = false;
        const row = step.querySelector('.tc-step__row--clickable');
        if (row) row.setAttribute('aria-expanded', 'true');
      }
    });
  });

  messagesEl.querySelectorAll('.tc-step__row--clickable').forEach(row => {
    const toggle = () => {
      const step = row.closest('.tc-step');
      const detail = step.querySelector('.tc-step__detail');
      const willExpand = detail.hidden;
      detail.hidden = !willExpand;
      row.setAttribute('aria-expanded', String(willExpand));
      step.classList.toggle('tc-step--open', willExpand);
    };
    row.addEventListener('click', toggle);
    row.addEventListener('keydown', e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggle(); } });
  });

  addCopyButtons(messagesEl, '.message__body pre');
  mountInlineGraphs(messagesEl);
}

function renderMessage(msg) {
  if (msg.role === 'user') {
    return `
      <div class="message message--user">
        <div class="message__bubble">${esc(msg.text)}</div>
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
          <strong>Error:</strong> ${esc(msg.error)}
        </div>
      </div>
    `;
  }

  const n = msg.tool_calls.length;
  const anyRunning = msg.tool_calls.some(tc => tc.status === 'running');
  const toolCallsHtml = n > 0 ? `
    <details class="tc-group${anyRunning ? ' tc-group--running' : ''}">
      <summary class="tc-group__summary">
        <svg class="tc-group__summary-chevron" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="2,3 5,7 8,3"/></svg>
        <span>${n} tool call${n === 1 ? '' : 's'}</span>
      </summary>
      ${msg.tool_calls.map((tc, i) => renderToolCall(tc, i)).join('')}
    </details>
  ` : '';

  const citationIndex = Object.fromEntries(
    (msg.sources || []).map((s, i) => [`${s.repo}:${s.version}:${s.file}:${s.line}`, i + 1])
  );

  const bodyHtml = msg.answer ? `
    <div class="message__body">${renderMarkdown(msg.answer, repoUrlMap, citationIndex)}</div>
  ` : (msg.status === 'loading' ? `
    <div class="loading-dots"><span></span><span></span><span></span></div>
  ` : '');

  const sourcesHtml = (msg.sources && msg.sources.length > 0) ? `
    <div class="sources">
      <div class="sources__title">Sources</div>
      <div class="source-chips">
        ${msg.sources.map((s, i) => {
          const repoUrl = repoUrlMap[s.repo];
          const fileUrl = repoUrl ? buildFileUrl(repoUrl, s.version, s.file, s.line) : null;
          const title = esc(`${s.repo} ${s.version} · ${s.file}:${s.line}`);
          const filename = esc(s.file.split('/').pop());
          const desc = esc(`${s.repo} · L${s.line}`);
          const inner = `<span class="source-chip__num">${i + 1}</span><strong class="source-chip__name">${filename}</strong><span class="source-chip__desc">${desc}</span>`;
          return fileUrl
            ? `<a href="${esc(fileUrl)}" class="source-chip" target="_blank" rel="noopener noreferrer" title="${title}">${inner}</a>`
            : `<span class="source-chip" title="${title}">${inner}</span>`;
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

function renderToolCall(tc, i) {
  const label = tc.description
    ? esc(tc.description)
    : `<span class="tc-step__name-bare">${esc(tc.name.replace(/_/g, ' '))}</span>`;

  const hasDetail = !!(tc.input || tc.preview);
  const detailHtml = hasDetail ? `
    <div class="tc-step__detail" hidden>
      <span class="tc-step__tool-tag">${esc(tc.name)}</span>
      ${tc.input ? `
        <div class="tc-step__detail-section">
          <div class="tc-step__detail-label">Input</div>
          <div class="tool-data">${renderJsonToHtml(tc.input)}</div>
        </div>` : ''}
      ${tc.preview ? `
        <div class="tc-step__detail-section">
          <div class="tc-step__detail-label">Result</div>
          <div class="tool-data">${renderPreviewToHtml(tc.preview, tc.input?.file ?? null)}</div>
        </div>` : ''}
    </div>
  ` : '';

  return `
    <div class="tc-step tc-step--${tc.status}" data-tc-idx="${i}">
      <div class="tc-step__row${hasDetail ? ' tc-step__row--clickable' : ''}" ${hasDetail ? 'role="button" tabindex="0" aria-expanded="false"' : ''}>
        <i class="p-icon--circle-of-friends tc-step__icon${tc.status === 'running' && !tc.description ? ' tc-step__icon--spinning' : ''}" aria-hidden="true"></i>
        <span class="tc-step__label">${label}</span>
        ${hasDetail ? `
          <svg class="tc-step__chevron" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg" aria-hidden="true"><polyline points="2,3 5,7 8,3"/></svg>` : ''}
      </div>
      ${detailHtml}
    </div>
  `;
}

async function sendQuery() {
  const query = inputEl.value.trim();
  if (!query || isLoading(state)) return;

  inputEl.value = '';

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
    saveCurrentConversation().catch(() => {});
  } catch (err) {
    state = setError(state, err.message);
    render();
  }
}

sendBtn.addEventListener('click', sendQuery);

inputEl.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    e.preventDefault();
    sendQuery();
  }
});

function updateThemeButton() {
  const t = getTheme();
  const label = getThemeLabel(t);
  const svg = getThemeIcon(t).replace(/^<svg /, '<svg class="p-side-navigation__icon" ');
  themeBtnEl.innerHTML = `${svg}<span class="p-side-navigation__label">${label}</span>`;
  themeBtnEl.setAttribute('aria-label', `Switch theme (current: ${label})`);
  themeBtnEl.title = label;
}

themeBtnEl.addEventListener('click', () => {
  nextTheme();
  updateThemeButton();
});

function openNav() {
  navEl.classList.remove('is-collapsed');
  navToggleEl.setAttribute('aria-expanded', 'true');
}

function closeNav() {
  // Vanilla's .l-navigation.is-collapsed:focus-within { transform: none } rule
  // keeps the nav visible if anything inside it still has focus when we collapse.
  // Blur first so that rule never fires.
  if (navEl.contains(document.activeElement)) {
    document.activeElement.blur();
  }
  navEl.classList.add('is-collapsed');
  navToggleEl.setAttribute('aria-expanded', 'false');
}

navToggleEl.addEventListener('click', openNav);
navCloseEl.addEventListener('click', closeNav);

document.addEventListener('click', (e) => {
  if (window.innerWidth < 620 &&
      !navEl.classList.contains('is-collapsed') &&
      !navEl.contains(e.target) &&
      !navToggleEl.contains(e.target)) {
    closeNav();
  }
});

document.querySelectorAll('#app-sidebar .p-side-navigation__link[data-page]').forEach(link => {
  link.addEventListener('click', (e) => {
    e.preventDefault();
    const page = link.dataset.page;

    document.querySelectorAll('#app-sidebar .p-side-navigation__link[data-page]').forEach(l => {
      l.removeAttribute('aria-current');
    });
    link.setAttribute('aria-current', 'page');

    const prevPage = document.querySelector('.page:not([hidden])');
    if (prevPage?.id === 'page-repositories') onRepositoriesPageHide();
    closeSourcePanel();
    if (page === 'admin' && !isAdmin()) return;

    document.querySelectorAll('.page').forEach(p => { p.hidden = true; });
    document.getElementById(`page-${page}`).hidden = false;

    if (page === 'repositories') onRepositoriesPageShow();

    if (window.innerWidth < 620) closeNav();
  });
});

applyStoredTheme();
updateThemeButton();
initSourcePanel();

const docsPage = initDocumentationPage(
  document.getElementById('page-documentation'),
  [],
);

// ── Auth guard ────────────────────────────────────────────────────────────────

const mainEl    = document.querySelector('.l-main');
const navEl2    = document.getElementById('app-sidebar');
const adminNavItem = document.getElementById('nav-item-admin');
const logoutBtn = document.getElementById('logout-btn');

// ── Conversation history ──────────────────────────────────────────────────────

const convListEl      = document.getElementById('conv-list');
const newChatBtn      = document.getElementById('new-chat-btn');
const historyPanel    = document.getElementById('chat-history-panel');
const historyToggle   = document.getElementById('chat-history-toggle');
const historyClose    = document.getElementById('chat-history-close');
const historyBackdrop = document.getElementById('chat-history-backdrop');

function openHistoryPanel() {
  historyPanel.classList.add('is-open');
  historyBackdrop.hidden = false;
  historyToggle?.setAttribute('aria-expanded', 'true');
}

function closeHistoryPanel() {
  historyPanel.classList.remove('is-open');
  historyBackdrop.hidden = true;
  historyToggle?.setAttribute('aria-expanded', 'false');
}

historyToggle?.addEventListener('click', () => {
  historyPanel.classList.contains('is-open') ? closeHistoryPanel() : openHistoryPanel();
});
historyClose?.addEventListener('click', closeHistoryPanel);
historyBackdrop?.addEventListener('click', closeHistoryPanel);

function refreshConvList() {
  renderConvList(convListEl, conversations, activeConvId, {
    onSelect: loadConversation,
    onDelete: handleDeleteConversation,
  });
}

async function saveCurrentConversation() {
  const saveable = getSaveableMessages(state);
  if (!saveable.length) return;

  const firstUser = saveable.find(m => m.role === 'user');
  const raw = firstUser?.text ?? 'New conversation';
  const title = raw.length > 60 ? raw.slice(0, 57) + '…' : raw;

  if (!activeConvId) {
    const conv = await createConversation(title);
    activeConvId = conv.id;
    conversations = [conv, ...conversations];
  }

  await updateConversation(activeConvId, { title, messages: saveable });

  const idx = conversations.findIndex(c => c.id === activeConvId);
  if (idx !== -1) {
    conversations[idx] = { ...conversations[idx], title, updated_at: new Date().toISOString() };
    conversations.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  }
  refreshConvList();
}

async function loadConversation(id) {
  try {
    const conv = await getConversation(id);
    activeConvId = id;
    state = loadFromHistory(Array.isArray(conv.messages) ? conv.messages : []);
    render();
    refreshConvList();
    closeHistoryPanel();
  } catch {}
}

async function handleDeleteConversation(id) {
  if (!confirm('Delete this conversation?')) return;
  try {
    await deleteConversation(id);
    conversations = conversations.filter(c => c.id !== id);
    if (activeConvId === id) {
      activeConvId = null;
      state = createChatState();
      render();
    }
    refreshConvList();
  } catch {}
}

newChatBtn?.addEventListener('click', () => {
  activeConvId = null;
  state = createChatState();
  render();
  refreshConvList();
});

// ── App init ──────────────────────────────────────────────────────────────────

function showApp(user) {
  setUser(user);
  hideAuthPages();
  mainEl.hidden = false;
  navEl2.hidden = false;

  // Show admin nav item only for admins
  if (adminNavItem) adminNavItem.hidden = user.role !== 'admin';

  // Init admin page if present
  const adminPage = document.getElementById('page-admin');
  if (adminPage && user.role === 'admin') initAdminPage(adminPage);

  // Load conversation history
  listConversations()
    .then(list => { conversations = list; refreshConvList(); })
    .catch(() => {});

  fetchRepositories().then((repos) => {
    repoUrlMap = Object.fromEntries(
      repos.filter(r => r.url).map(r => [r.name, r.url])
    );
    initRepositoriesPage(repos);
    docsPage.update(repos);
  });
  render();
}

function showAuth() {
  mainEl.hidden = true;
  navEl2.hidden = true;
  showLoginPage();
}

setUnauthorizedHandler(showAuth);

initAuthPages({ onLoginSuccess: showApp });

logoutBtn?.addEventListener('click', async () => {
  await logout();
  window.location.reload();
});

fetchMe().then(user => {
  if (user) {
    showApp(user);
  } else {
    showAuth();
  }
});

import './vanilla.scss';
import './style.css';
import { applyStoredTheme, nextTheme, getTheme, getThemeIcon, getThemeLabel } from './theme.js';
import {
  queryStream, fetchToolDescription, fetchRepositories, setUnauthorizedHandler,
  projectQueryStart, openProjectEvents,
  listProjectConversations, createProjectConversation,
  getProjectConversation, updateProjectConversation, deleteProjectConversation,
} from './api.js';
import { initRepositoriesPage, onRepositoriesPageShow, onRepositoriesPageHide } from './repositories.js';
import { initAgentsPage, onAgentsPageShow, onAgentsPageHide } from './agents.js';
import { initSecretsPage, onSecretsPageShow, onSecretsPageHide } from './secrets.js';
import { initDocumentationPage } from './documentation.js';
import { renderMarkdown, buildFileUrl } from './markdown.js';
import { renderJsonToHtml, renderPreviewToHtml } from './format.js';
import { escapeHtml as esc, addCopyButtons, avatarColor, initials } from './utils.js';
import { initSourcePanel, closeSourcePanel } from './source-panel.js';
import { initProjectSelector, getCurrentProject, refreshProjectSelector } from './projects.js';
import { mountInlineGraphs } from './inline-graph.js';
import {
  fetchMe, logout, initAuthPages, showLoginPage, hideAuthPages, isAdmin, setUser, getUser,
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
  addPendingAttachment,
  removePendingAttachment,
  getPendingAttachments,
  clearPendingAttachments,
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
let activeProjectId = null; // null = personal context

// Project real-time collaboration state
let projectEventSource = null;
let projectRemoteLocked = false;   // locked by someone else
let projectLockedBy = '';
let projectPresence = [];          // [{user_id, name}]
let lastProjectQuery = '';         // last query sent to project, for conv list update

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

  const atBottom = messagesEl.scrollHeight - messagesEl.scrollTop - messagesEl.clientHeight < 80;
  const prevScrollTop = messagesEl.scrollTop;

  const messages = getMessages(state);
  messagesEl.innerHTML = messages.map(renderMessage).join('');

  const loading = isLoading(state);
  sendBtn.disabled = !activeProjectId || loading;
  inputEl.disabled = !activeProjectId || loading;

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

  if (atBottom) {
    messagesEl.scrollTop = messagesEl.scrollHeight;
  } else {
    messagesEl.scrollTop = prevScrollTop;
  }

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

function renderAttachments(attachments) {
  if (!attachments?.length) return '';
  return `<div class="message__attachments">${attachments.map(a => {
    if (a.preview_url) {
      return `<img class="message__attachment-img" src="${esc(a.preview_url)}" alt="${esc(a.name)}" title="${esc(a.name)}">`;
    }
    return `<span class="message__attachment-chip"><i class="p-icon--document" aria-hidden="true"></i>${esc(a.name)}</span>`;
  }).join('')}</div>`;
}

function renderMessage(msg) {
  if (msg.role === 'user') {
    const senderHtml = msg.username ? `
      <div class="message__sender">
        <div class="message-avatar" style="background:${esc(avatarColor(msg.username))}"
          aria-hidden="true">${esc(initials(msg.username))}</div>
        <span class="message__sender-name">${esc(msg.username)}</span>
      </div>` : '';
    return `
      <div class="message message--user">
        ${senderHtml}
        ${renderAttachments(msg.attachments)}
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

// ── Attachment helpers ────────────────────────────────────────────────────────

const attachTrayEl   = document.getElementById('attachment-tray');
const attachInputEl  = document.getElementById('attachment-input');
const attachBtnEl    = document.getElementById('attach-btn');

function toWireAttachment(a) {
  return { name: a.name, mime_type: a.mime_type, data: a.data };
}

function renderAttachmentTray() {
  if (!attachTrayEl) return;
  const pending = getPendingAttachments(state);
  if (pending.length === 0) { attachTrayEl.hidden = true; attachTrayEl.innerHTML = ''; return; }
  attachTrayEl.hidden = false;
  attachTrayEl.innerHTML = pending.map(a => {
    const thumb = a.preview_url
      ? `<img class="attach-chip__thumb" src="${esc(a.preview_url)}" alt="">`
      : `<i class="p-icon--document" aria-hidden="true"></i>`;
    return `<div class="attach-chip" data-id="${a.id}">
      ${thumb}
      <span class="attach-chip__name">${esc(a.name)}</span>
      <button class="attach-chip__remove" aria-label="Remove ${esc(a.name)}" data-id="${a.id}">×</button>
    </div>`;
  }).join('');
}

async function addFileAttachment(file) {
  const ACCEPTED = ['image/png','image/jpeg','image/gif','image/webp','application/pdf'];
  if (!ACCEPTED.includes(file.type)) return;
  const data = await new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result.split(',')[1]);
    reader.onerror = reject;
    reader.readAsDataURL(file);
  });
  const preview_url = file.type.startsWith('image/') ? `data:${file.type};base64,${data}` : null;
  state = addPendingAttachment(state, { name: file.name, mime_type: file.type, data, preview_url });
  renderAttachmentTray();
}

if (attachBtnEl) {
  attachBtnEl.addEventListener('click', () => attachInputEl?.click());
}
if (attachInputEl) {
  attachInputEl.addEventListener('change', (e) => {
    [...(e.target.files ?? [])].forEach(addFileAttachment);
    attachInputEl.value = '';
  });
}
document.addEventListener('paste', (e) => {
  if (!activeProjectId && !document.getElementById('page-chat')?.offsetParent) return;
  const items = [...(e.clipboardData?.items ?? [])];
  for (const item of items) {
    if (item.kind === 'file') {
      const file = item.getAsFile();
      if (file) addFileAttachment(file);
    }
  }
});
if (attachTrayEl) {
  attachTrayEl.addEventListener('click', (e) => {
    const btn = e.target.closest('.attach-chip__remove');
    if (!btn) return;
    state = removePendingAttachment(state, Number(btn.dataset.id));
    renderAttachmentTray();
  });
}

async function sendQuery() {
  const query = inputEl.value.trim();
  const hasPending = getPendingAttachments(state).length > 0;
  if ((!query && !hasPending) || isLoading(state) || projectRemoteLocked) return;
  inputEl.value = '';

  if (activeProjectId) {
    // Project chat: fire-and-forget POST; all events arrive via EventSource.
    // Ensure a conversation exists before querying so the server can save the turn.
    const attachments = getPendingAttachments(state).map(toWireAttachment);
    state = clearPendingAttachments(state);
    renderAttachmentTray();
    try {
      if (!activeConvId) {
        const title = query.length > 60 ? query.slice(0, 57) + '…' : query;
        const conv = await createProjectConversation(activeProjectId, { title });
        activeConvId = conv.id;
        conversations = [conv, ...conversations];
        resubscribeProjectEvents();
      }
      lastProjectQuery = query;
      await projectQueryStart(activeProjectId, query, activeConvId, attachments);
    } catch (err) {
      if (err.status === 409) {
        // Already locked — the lock banner will have appeared via EventSource.
      } else {
        state = addUserMessage(state, query, getUser()?.name ?? null, attachments);
        state = startAssistantMessage(state);
        state = setError(state, err.message);
        render();
      }
    }
    return;
  }

  // Personal chat: ensure a conversation exists before querying.
  const attachments = getPendingAttachments(state).map(toWireAttachment);
  state = clearPendingAttachments(state);
  renderAttachmentTray();
  state = addUserMessage(state, query, null, attachments);
  state = startAssistantMessage(state);
  render();

  try {
    if (!activeConvId) {
      const title = query.length > 60 ? query.slice(0, 57) + '…' : query;
      const conv = await createConversation(title);
      activeConvId = conv.id;
      conversations = [conv, ...conversations];
      refreshConvList();
    }
    await queryStream(query, activeConvId, attachments, (event) => {
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
        updateConvListEntry(query);
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

// ── Project presence & real-time event handling ───────────────────────────────

function renderPresence() {
  const bar = document.getElementById('project-presence-bar');
  if (!bar) return;
  const me = getUser();
  // Only show users viewing the exact same conversation.
  const others = projectPresence.filter(u =>
    u.user_id !== me?.id &&
    u.conv_id != null &&
    u.conv_id === activeConvId
  );
  if (!activeProjectId || !activeConvId || others.length === 0) {
    bar.hidden = true;
    return;
  }
  bar.hidden = false;
  bar.innerHTML = others.map(u => `
    <div class="presence-avatar" title="${esc(u.name)}"
      style="background:${esc(avatarColor(u.name))}"
      aria-label="${esc(u.name)}">
      ${esc(initials(u.name))}
    </div>`).join('');
}

function renderLockBanner() {
  const banner = document.getElementById('project-lock-banner');
  if (!banner) return;
  if (projectRemoteLocked) {
    banner.textContent = `${projectLockedBy} is generating a response…`;
    banner.hidden = false;
  } else {
    banner.hidden = true;
  }
}

function handleProjectEvent(event) {
  switch (event.type) {
    case 'presence':
      projectPresence = event.users ?? [];
      renderPresence();
      break;

    case 'user_join':
      projectPresence = [
        ...projectPresence.filter(u => u.user_id !== event.user_id),
        { user_id: event.user_id, name: event.name, conv_id: event.conv_id ?? null },
      ];
      renderPresence();
      break;

    case 'user_leave':
      projectPresence = projectPresence.filter(u => u.user_id !== event.user_id);
      renderPresence();
      break;

    case 'lock':
      projectRemoteLocked = true;
      projectLockedBy = event.by ?? '';
      renderLockBanner();
      sendBtn.disabled = true;
      inputEl.disabled = true;
      break;

    case 'unlock':
      projectRemoteLocked = false;
      projectLockedBy = '';
      renderLockBanner();
      if (!isLoading(state)) {
        sendBtn.disabled = false;
        inputEl.disabled = false;
      }
      break;

    case 'user_message': {
      const msgUsername = event.username ?? null;
      state = addUserMessage(state, event.query ?? '', msgUsername, event.attachments ?? []);
      state = startAssistantMessage(state);
      render();
      break;
    }

    case 'tool_call':
      state = addToolCall(state, { name: event.name, input: event.input });
      fetchToolDescription(event.name, event.input).then(description => {
        if (description) {
          const tc = getMessages(state).at(-1)?.tool_calls?.at(-1);
          if (tc) { state = updateToolCallDescription(state, { id: tc.id, description }); render(); }
        }
      });
      render();
      break;

    case 'tool_result':
      state = completeToolCall(state, { name: event.name, preview: event.preview });
      render();
      break;

    case 'done':
      state = finalizeAssistantMessage(state, {
        answer: event.answer,
        sources: event.sources,
        tool_calls_made: event.tool_calls_made,
      });
      render();
      updateConvListEntry(lastProjectQuery);
      break;

    case 'error':
      state = setError(state, event.message);
      render();
      break;
  }
}

function openProjectEventStream(projectId, convId = null) {
  closeProjectEventStream();
  projectEventSource = openProjectEvents(projectId, convId, handleProjectEvent);
}

function resubscribeProjectEvents() {
  if (activeProjectId) openProjectEventStream(activeProjectId, activeConvId);
}

function closeProjectEventStream() {
  if (projectEventSource) {
    projectEventSource.close();
    projectEventSource = null;
  }
  projectRemoteLocked = false;
  projectLockedBy = '';
  projectPresence = [];
  renderPresence();
  renderLockBanner();
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
    if (prevPage?.id === 'page-agents')       onAgentsPageHide();
    if (prevPage?.id === 'page-secrets')      onSecretsPageHide();
    closeSourcePanel();
    if (page === 'admin' && !isAdmin()) return;

    document.querySelectorAll('.page').forEach(p => { p.hidden = true; });
    document.getElementById(`page-${page}`).hidden = false;

    if (page === 'repositories') onRepositoriesPageShow();
    if (page === 'agents')       onAgentsPageShow(activeProjectId);
    if (page === 'secrets')      onSecretsPageShow(activeProjectId);

    if (window.innerWidth < 620) closeNav();
  });
});

applyStoredTheme();
updateThemeButton();
initSourcePanel();
initProjectSelector(document.getElementById('project-selector-section'), {
  onChange: switchToProject,
});

const docsPage = initDocumentationPage(
  document.getElementById('page-documentation'),
  [],
);

initAgentsPage(document.getElementById('page-agents'));
initSecretsPage(document.getElementById('page-secrets'));

const mainEl    = document.querySelector('.l-main');
const navEl2    = document.getElementById('app-sidebar');
const adminNavItem = document.getElementById('nav-item-admin');
const logoutBtn = document.getElementById('logout-btn');

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
  const user = getUser();
  renderConvList(convListEl, conversations, activeConvId, {
    onSelect:    loadConversation,
    onDelete:    handleDeleteConversation,
    showCreator: !!activeProjectId,
    canDelete:   (c) => !activeProjectId || isAdmin() || c.created_by === user?.id,
  });
}

function updateConvListEntry(queryText) {
  if (!activeConvId) return;
  const idx = conversations.findIndex(c => c.id === activeConvId);
  if (idx !== -1) {
    const raw   = queryText ?? conversations[idx].title ?? 'New conversation';
    const title = raw.length > 60 ? raw.slice(0, 57) + '…' : raw;
    conversations[idx] = { ...conversations[idx], title, updated_at: new Date().toISOString() };
    conversations.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  }
  refreshConvList();
}

async function switchToProject(project) {
  activeProjectId = project?.id ?? null;
  activeConvId    = null;
  state           = createChatState();
  render();

  // Refresh agents/secrets pages if currently visible
  const agentsPage = document.getElementById('page-agents');
  if (agentsPage && !agentsPage.hidden) {
    onAgentsPageHide();
    onAgentsPageShow(activeProjectId);
  }
  onSecretsPageShow(activeProjectId);

  const hasProject = !!activeProjectId;

  const header = document.getElementById('project-chat-header');
  if (header) header.hidden = !hasProject;

  const noProject = document.getElementById('chat-no-project');
  if (noProject) noProject.hidden = hasProject;

  if (historyPanel) historyPanel.hidden = !hasProject;
  if (historyToggle) historyToggle.hidden = !hasProject;
  if (!hasProject) closeHistoryPanel();

  inputEl.disabled = !hasProject;
  sendBtn.disabled = !hasProject;

  if (hasProject) {
    openProjectEventStream(activeProjectId);
  } else {
    closeProjectEventStream();
    conversations = [];
    refreshConvList();
    return;
  }

  try {
    conversations = await listProjectConversations(activeProjectId);
  } catch {
    conversations = [];
  }
  refreshConvList();
}

async function saveCurrentConversation() {
  const saveable = getSaveableMessages(state);
  if (!saveable.length) return;

  const firstUser = saveable.find(m => m.role === 'user');
  const raw   = firstUser?.text ?? 'New conversation';
  const title = raw.length > 60 ? raw.slice(0, 57) + '…' : raw;

  if (activeProjectId) {
    const user     = getUser();
    const username = user?.name ?? user?.email ?? 'Unknown';
    const projectMessages = saveable.map(m =>
      m.role === 'user' && !m.username ? { ...m, username } : m
    );

    if (!activeConvId) {
      const conv = await createProjectConversation(activeProjectId, { title });
      activeConvId = conv.id;
      conversations = [conv, ...conversations];
      resubscribeProjectEvents();
    }
    await updateProjectConversation(activeProjectId, activeConvId, { title, messages: projectMessages });
  } else {
    if (!activeConvId) {
      const conv = await createConversation(title);
      activeConvId = conv.id;
      conversations = [conv, ...conversations];
    }
    await updateConversation(activeConvId, { title, messages: saveable });
  }

  const idx = conversations.findIndex(c => c.id === activeConvId);
  if (idx !== -1) {
    conversations[idx] = { ...conversations[idx], title, updated_at: new Date().toISOString() };
    conversations.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  }
  refreshConvList();
}

async function loadConversation(id) {
  try {
    const conv = activeProjectId
      ? await getProjectConversation(activeProjectId, id)
      : await getConversation(id);
    activeConvId = id;
    state = loadFromHistory(Array.isArray(conv.messages) ? conv.messages : []);
    render();
    messagesEl.scrollTop = messagesEl.scrollHeight;
    refreshConvList();
    closeHistoryPanel();
    resubscribeProjectEvents();
  } catch {}
}

async function handleDeleteConversation(id) {
  if (!confirm('Delete this conversation?')) return;
  try {
    if (activeProjectId) {
      await deleteProjectConversation(activeProjectId, id);
    } else {
      await deleteConversation(id);
    }
    conversations = conversations.filter(c => c.id !== id);
    if (activeConvId === id) {
      activeConvId = null;
      state = createChatState();
      render();
    }
    refreshConvList();
  } catch (err) {
    alert(`Could not delete: ${err.message}`);
  }
}

newChatBtn?.addEventListener('click', () => {
  activeConvId = null;
  state = createChatState();
  render();
  refreshConvList();
  resubscribeProjectEvents();
});

function showApp(user) {
  setUser(user);
  hideAuthPages();
  mainEl.hidden = false;
  navEl2.hidden = false;

  if (adminNavItem) adminNavItem.hidden = user.role !== 'admin';

  const adminPage = document.getElementById('page-admin');
  if (adminPage && user.role === 'admin') initAdminPage(adminPage, { onUserGroupsChanged: refreshProjectSelector });

  refreshProjectSelector();

  activeProjectId = null;
  const noProject = document.getElementById('chat-no-project');
  if (noProject) noProject.hidden = false;
  if (historyPanel) historyPanel.hidden = true;
  if (historyToggle) historyToggle.hidden = true;
  inputEl.disabled = true;
  sendBtn.disabled = true;
  conversations = [];
  refreshConvList();

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
  closeProjectEventStream();
  await logout();
  window.location.reload();
});

function dismissLoading() {
  const el = document.getElementById('app-loading');
  if (!el) return;
  el.style.opacity = '0';
  el.addEventListener('transitionend', () => el.remove(), { once: true });
}

fetchMe().then(user => {
  dismissLoading();
  if (user) {
    showApp(user);
  } else {
    showAuth();
  }
});

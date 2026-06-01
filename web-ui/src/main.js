import './vanilla.scss';
import './style.css';
import { applyStoredTheme, nextTheme, getTheme, getThemeIcon, getThemeLabel } from './theme.js';
import { queryStream, fetchToolDescription, fetchRepositories } from './api.js';
import { initRepositoriesPage, onRepositoriesPageShow, onRepositoriesPageHide } from './repositories.js';
import { initDocumentationPage } from './documentation.js';
import { renderMarkdown, buildFileUrl } from './markdown.js';
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

const messagesEl  = document.getElementById('messages');
const inputEl     = document.getElementById('query-input');
const sendBtn     = document.getElementById('send-btn');
const themeBtnEl  = document.getElementById('theme-btn');
const navEl       = document.getElementById('app-sidebar');
const navToggleEl = document.getElementById('nav-toggle');
const navCloseEl  = document.getElementById('nav-close');

// ── Render ────────────────────────────────────────────────────────────────────

function render() {
  // Snapshot open state before replacing the DOM
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

  // Scroll to bottom
  messagesEl.scrollTop = messagesEl.scrollHeight;

  // Restore open state
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

  // Attach step detail expand/collapse listeners (whole row is the trigger)
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

  // Wrap each code block in a container and inject a copy button
  messagesEl.querySelectorAll('.message__body pre').forEach(pre => {
    const wrapper = document.createElement('div');
    wrapper.className = 'code-block';
    pre.parentNode.insertBefore(wrapper, pre);
    wrapper.appendChild(pre);

    const btn = document.createElement('button');
    btn.className = 'copy-btn';
    btn.textContent = 'Copy';
    btn.addEventListener('click', () => {
      const text = (pre.querySelector('code') ?? pre).innerText;
      copyText(text).then(() => {
        btn.textContent = 'Copied!';
      }).catch(() => {
        btn.textContent = 'Failed';
      }).finally(() => {
        setTimeout(() => { btn.textContent = 'Copy'; }, 2000);
      });
    });
    wrapper.appendChild(btn);
  });
}

function copyText(text) {
  if (navigator.clipboard?.writeText) {
    return navigator.clipboard.writeText(text);
  }
  // Fallback for non-secure contexts (HTTP on non-localhost)
  return new Promise((resolve, reject) => {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.cssText = 'position:fixed;opacity:0;pointer-events:none';
    document.body.appendChild(ta);
    ta.focus();
    ta.select();
    try {
      document.execCommand('copy') ? resolve() : reject(new Error('execCommand failed'));
    } catch (e) {
      reject(e);
    } finally {
      document.body.removeChild(ta);
    }
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

  // Map each unique source to a 1-based index so inline citations and chips share the same numbering
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
          const title = escapeHtml(`${s.repo} ${s.version} · ${s.file}:${s.line}`);
          const filename = escapeHtml(s.file.split('/').pop());
          const desc = escapeHtml(`${s.repo} · L${s.line}`);
          const inner = `<span class="source-chip__num">${i + 1}</span><strong class="source-chip__name">${filename}</strong><span class="source-chip__desc">${desc}</span>`;
          return fileUrl
            ? `<a href="${escapeHtml(fileUrl)}" class="source-chip" target="_blank" rel="noopener noreferrer" title="${title}">${inner}</a>`
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
    ? escapeHtml(tc.description)
    : `<span class="tc-step__name-bare">${escapeHtml(tc.name.replace(/_/g, ' '))}</span>`;

  const hasDetail = !!(tc.input || tc.preview);
  const detailHtml = hasDetail ? `
    <div class="tc-step__detail" hidden>
      <span class="tc-step__tool-tag">${escapeHtml(tc.name)}</span>
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
        <i class="p-icon--circle-of-friends tc-step__icon${tc.description ? '' : ' tc-step__icon--spinning'}" aria-hidden="true"></i>
        <span class="tc-step__label">${label}</span>
        ${hasDetail ? `
          <svg class="tc-step__chevron" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg" aria-hidden="true"><polyline points="2,3 5,7 8,3"/></svg>` : ''}
      </div>
      ${detailHtml}
    </div>
  `;
}

// ── Send ──────────────────────────────────────────────────────────────────────

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
  } catch (err) {
    state = setError(state, err.message);
    render();
  }
}

// ── Event listeners ───────────────────────────────────────────────────────────

sendBtn.addEventListener('click', sendQuery);

inputEl.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    e.preventDefault();
    sendQuery();
  }
});

// ── Theme ─────────────────────────────────────────────────────────────────────

function updateThemeButton() {
  const t = getTheme();
  const label = getThemeLabel(t);
  // Inject the SVG as a p-side-navigation__icon + label span
  const svg = getThemeIcon(t).replace(/^<svg /, '<svg class="p-side-navigation__icon" ');
  themeBtnEl.innerHTML = `${svg}<span class="p-side-navigation__label">${label}</span>`;
  themeBtnEl.setAttribute('aria-label', `Switch theme (current: ${label})`);
  themeBtnEl.title = label;
}

themeBtnEl.addEventListener('click', () => {
  nextTheme();
  updateThemeButton();
});

// ── Navigation open / close (mobile) ──────────────────────────────────────────

function openNav() {
  navEl.classList.remove('is-collapsed');
  navToggleEl.setAttribute('aria-expanded', 'true');
}

function closeNav() {
  navEl.classList.add('is-collapsed');
  navToggleEl.setAttribute('aria-expanded', 'false');
}

navToggleEl.addEventListener('click', openNav);
navCloseEl.addEventListener('click', closeNav);

// Close nav on mobile when clicking outside it
document.addEventListener('click', (e) => {
  if (window.innerWidth < 620 &&
      !navEl.classList.contains('is-collapsed') &&
      !navEl.contains(e.target) &&
      !navToggleEl.contains(e.target)) {
    closeNav();
  }
});

// ── Sidebar navigation ────────────────────────────────────────────────────────

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

    document.querySelectorAll('.page').forEach(p => { p.hidden = true; });
    document.getElementById(`page-${page}`).hidden = false;

    if (page === 'repositories') onRepositoriesPageShow();

    // Close drawer on mobile after navigating
    if (window.innerWidth < 620) closeNav();
  });
});

// ── Bootstrap ─────────────────────────────────────────────────────────────────

applyStoredTheme();
updateThemeButton();

const docsPage = initDocumentationPage(
  document.getElementById('page-documentation'),
  [],
);

fetchRepositories().then((repos) => {
  repoUrlMap = Object.fromEntries(
    repos.filter(r => r.url).map(r => [r.name, r.url])
  );
  initRepositoriesPage(repos);
  docsPage.update(repos);
});
render();

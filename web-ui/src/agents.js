import { listProjectAgents, rotateInstallToken } from './api.js';
import { escapeHtml as esc } from './utils.js';

let _projectId       = null;
let _serverUrl       = window.location.origin;
let _refreshInterval = null;

// ── Public API ────────────────────────────────────────────────────────────────

export function initAgentsPage(pageEl) {
  const installBtn      = document.getElementById('install-agent-btn');
  const modal           = document.getElementById('install-modal');
  const modalClose      = document.getElementById('install-modal-close');
  const copyBtn         = document.getElementById('copy-install-cmd');
  const rotateBtn       = document.getElementById('rotate-install-token-btn');

  installBtn?.addEventListener('click', () => openInstallModal());
  modalClose?.addEventListener('click', closeInstallModal);
  copyBtn?.addEventListener('click', copyInstallCommand);
  rotateBtn?.addEventListener('click', handleRotate);

  // Close modal on backdrop click
  modal?.addEventListener('click', e => {
    if (e.target === modal) closeInstallModal();
  });

  // Close modal on Escape
  document.addEventListener('keydown', e => {
    if (e.key === 'Escape' && !modal?.hidden) closeInstallModal();
  });
}

export function onAgentsPageShow(projectId) {
  _projectId = projectId;

  const installBtn   = document.getElementById('install-agent-btn');
  const noProject    = document.getElementById('agents-no-project');
  const content      = document.getElementById('agents-content');

  if (!projectId) {
    if (installBtn)  installBtn.hidden  = true;
    if (noProject)   noProject.hidden   = false;
    if (content)     content.hidden     = true;
    return;
  }

  if (installBtn)  installBtn.hidden  = false;
  if (noProject)   noProject.hidden   = true;
  if (content)     content.hidden     = false;

  loadAndRender();
  _refreshInterval = setInterval(loadAndRender, 15_000);
}

export function onAgentsPageHide() {
  clearInterval(_refreshInterval);
  _refreshInterval = null;
}

// ── Rendering ─────────────────────────────────────────────────────────────────

async function loadAndRender() {
  if (!_projectId) return;
  try {
    const agents = await listProjectAgents(_projectId);
    renderAgents(agents);
  } catch {
    // silently ignore refresh errors
  }
}

export function renderAgents(agents) {
  const tbody = document.getElementById('agents-tbody');
  const empty = document.getElementById('agents-empty');
  if (!tbody) return;

  if (!agents.length) {
    tbody.innerHTML = '';
    if (empty) empty.hidden = false;
    return;
  }

  if (empty) empty.hidden = true;
  tbody.innerHTML = agents.map(a => agentRow(a)).join('');
}

function agentRow(a) {
  const dot   = `<span class="agent-status__dot agent-status__dot--${a.online ? 'online' : 'offline'}"></span>`;
  const label = a.online ? 'Online' : 'Offline';
  const seen  = a.last_seen ? relativeTime(a.last_seen) : '—';
  const since = a.connected_at ? relativeTime(a.connected_at) : '—';
  return `<tr>
    <td><span class="agent-status">${dot}${esc(label)}</span></td>
    <td>${esc(a.hostname || a.id)}</td>
    <td>${esc(seen)}</td>
    <td>${a.online ? esc(since) : '—'}</td>
  </tr>`;
}

function relativeTime(iso) {
  const diff = Date.now() - new Date(iso).getTime();
  const s    = Math.floor(diff / 1000);
  if (s < 60)   return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

// ── Install modal ─────────────────────────────────────────────────────────────

function openInstallModal() {
  if (!_projectId) return;
  const modal   = document.getElementById('install-modal');
  const cmdEl   = document.getElementById('install-cmd');
  const cmd     = `curl -fsSL ${_serverUrl}/agents/${_projectId}/install.sh | sudo bash`;
  if (cmdEl)  cmdEl.textContent = cmd;
  if (modal)  modal.hidden = false;
}

function closeInstallModal() {
  const modal = document.getElementById('install-modal');
  if (modal) modal.hidden = true;
}

function copyInstallCommand() {
  const cmdEl = document.getElementById('install-cmd');
  if (!cmdEl) return;
  navigator.clipboard.writeText(cmdEl.textContent).then(() => {
    const btn = document.getElementById('copy-install-cmd');
    if (!btn) return;
    const orig = btn.textContent;
    btn.textContent = 'Copied!';
    setTimeout(() => { btn.textContent = orig; }, 2000);
  });
}

async function handleRotate() {
  if (!_projectId) return;
  if (!confirm('Rotate the install key? Existing agents will not be affected, but any unfinished install scripts will stop working.')) return;
  try {
    await rotateInstallToken(_projectId);
    alert('Install key rotated. Refresh the install command to get the new one.');
  } catch {
    alert('Failed to rotate install key.');
  }
}

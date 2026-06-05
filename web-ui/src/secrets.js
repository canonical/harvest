import { escapeHtml as esc } from './utils.js';

const BASE = (projectId) => `/projects/${projectId}/secrets`;

async function apiFetch(url, options = {}) {
  const res = await fetch(url, {
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `Request failed (${res.status})`);
  }
  if (res.status === 204) return null;
  return res.json();
}

let _projectId = null;

export function initSecretsPage(pageEl) {
  const addBtn    = document.getElementById('add-secret-btn');
  const modal     = document.getElementById('add-secret-modal');
  const closeBtn  = document.getElementById('add-secret-modal-close');
  const cancelBtn = document.getElementById('add-secret-cancel');
  const submitBtn = document.getElementById('add-secret-submit');
  const nameInput = document.getElementById('secret-name-input');
  const valInput  = document.getElementById('secret-value-input');
  const errorEl   = document.getElementById('add-secret-error');

  addBtn?.addEventListener('click', openModal);
  closeBtn?.addEventListener('click', closeModal);
  cancelBtn?.addEventListener('click', closeModal);
  modal?.addEventListener('click', e => { if (e.target === modal) closeModal(); });
  document.addEventListener('keydown', e => { if (e.key === 'Escape' && !modal?.hidden) closeModal(); });

  submitBtn?.addEventListener('click', async () => {
    const name  = nameInput?.value.trim();
    const value = valInput?.value;
    if (!name || !value) { if (errorEl) errorEl.textContent = 'Name and value are required.'; return; }
    if (errorEl) errorEl.textContent = '';
    try {
      await apiFetch(BASE(_projectId), { method: 'POST', body: JSON.stringify({ name, value }) });
      closeModal();
      await loadSecrets();
    } catch (err) {
      if (errorEl) errorEl.textContent = err.message;
    }
  });

  function openModal() {
    if (nameInput) nameInput.value = '';
    if (valInput)  valInput.value  = '';
    if (errorEl)   errorEl.textContent = '';
    if (modal)     modal.hidden = false;
    nameInput?.focus();
  }

  function closeModal() {
    if (modal) modal.hidden = true;
  }
}

export function onSecretsPageShow(projectId) {
  _projectId = projectId;

  const noProject  = document.getElementById('secrets-no-project');
  const content    = document.getElementById('secrets-content');
  const addBtn     = document.getElementById('add-secret-btn');
  const pageEl     = document.getElementById('page-secrets');

  if (!projectId) {
    if (noProject) noProject.hidden = false;
    if (content)   content.hidden   = true;
    if (addBtn)    addBtn.hidden     = true;
    return;
  }

  if (noProject) noProject.hidden = true;
  if (content)   content.hidden   = false;
  if (addBtn)    addBtn.hidden     = false;

  if (pageEl && !pageEl.hidden) loadSecrets();
}

export function onSecretsPageHide() {
  _projectId = null;
}

async function loadSecrets() {
  if (!_projectId) return;
  const tbody  = document.getElementById('secrets-tbody');
  const emptyEl = document.getElementById('secrets-empty');
  try {
    const secrets = await apiFetch(BASE(_projectId));
    renderSecrets(secrets, tbody, emptyEl);
  } catch {
    if (tbody) tbody.innerHTML = '';
    if (emptyEl) { emptyEl.textContent = 'Failed to load secrets.'; emptyEl.hidden = false; }
  }
}

function renderSecrets(secrets, tbody, emptyEl) {
  if (!tbody) return;
  if (!secrets?.length) {
    tbody.innerHTML = '';
    if (emptyEl) emptyEl.hidden = false;
    return;
  }
  if (emptyEl) emptyEl.hidden = true;
  tbody.innerHTML = secrets.map(s => `
    <tr>
      <td><code>${esc(s.name)}</code></td>
      <td>${esc(new Date(s.created_at).toLocaleString())}</td>
      <td>
        <button class="p-button--base is-small" data-delete-secret="${esc(s.name)}"
          aria-label="Delete ${esc(s.name)}">Delete</button>
      </td>
    </tr>
  `).join('');

  tbody.querySelectorAll('[data-delete-secret]').forEach(btn => {
    btn.addEventListener('click', async () => {
      const name = btn.dataset.deleteSecret;
      if (!confirm(`Delete secret "${name}"?`)) return;
      try {
        await apiFetch(`${BASE(_projectId)}/${encodeURIComponent(name)}`, { method: 'DELETE' });
        await loadSecrets();
      } catch (err) {
        alert(`Could not delete: ${err.message}`);
      }
    });
  });
}

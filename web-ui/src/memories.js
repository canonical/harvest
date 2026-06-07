import { listProjectMemories, createProjectMemory, getProjectMemory, updateProjectMemory, deleteProjectMemory } from './api.js';
import { renderMarkdown } from './markdown.js';
import { escapeHtml as esc } from './utils.js';

let _projectId = null;
let _memories = [];
let _selectedId = null;
let _selectedMemory = null;
let _loading = false;
let _contentLoading = false;

export function initMemoriesPage(pageEl) {
  pageEl.addEventListener('click', handlePageClick);
  document.addEventListener('keydown', handleKeydown);

  const addBtn = document.getElementById('add-memory-btn');
  addBtn?.addEventListener('click', openCreateModal);

  const modal       = document.getElementById('add-memory-modal');
  const closeBtn    = document.getElementById('add-memory-modal-close');
  const cancelBtn   = document.getElementById('add-memory-cancel');
  const submitBtn   = document.getElementById('add-memory-submit');

  closeBtn?.addEventListener('click', closeCreateModal);
  cancelBtn?.addEventListener('click', closeCreateModal);
  modal?.addEventListener('click', e => { if (e.target === modal) closeCreateModal(); });
  submitBtn?.addEventListener('click', submitCreate);
}

export async function onMemoriesPageShow(projectId) {
  _projectId = projectId;
  _selectedId = null;
  _selectedMemory = null;

  if (!projectId) {
    renderPage();
    return;
  }

  _loading = true;
  renderPage();

  try {
    _memories = await listProjectMemories(projectId);
  } catch {
    _memories = [];
  }
  _loading = false;
  renderPage();
}

export function onMemoriesPageHide() {
  _projectId = null;
  _memories = [];
  _selectedId = null;
  _selectedMemory = null;
}

function renderPage() {
  const pageEl = document.getElementById('page-memories');
  if (!pageEl) return;

  const noProject = document.getElementById('memories-no-project');
  const content   = document.getElementById('memories-content');
  const addBtn    = document.getElementById('add-memory-btn');

  if (!_projectId) {
    if (noProject) noProject.hidden = false;
    if (content)   content.hidden   = true;
    if (addBtn)    addBtn.hidden     = true;
    return;
  }

  if (noProject) noProject.hidden = true;
  if (content)   content.hidden   = false;
  if (addBtn)    addBtn.hidden     = false;

  const listEl    = document.getElementById('memories-list');
  const detailEl  = document.getElementById('memories-detail');
  if (!listEl || !detailEl) return;

  listEl.innerHTML   = renderList();
  detailEl.innerHTML = renderDetail();
}

function renderList() {
  if (_loading) {
    return `<div class="memories-list-loading"><div class="loading-dots"><span></span><span></span><span></span></div></div>`;
  }
  if (!_memories.length) {
    return `<p class="memories-list-empty">No memories yet. Create one to get started.</p>`;
  }
  return _memories.map(m => {
    const isActive = m.id === _selectedId;
    const date = new Date(m.created_at).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
    return `
      <button
        class="memories-list-item${isActive ? ' memories-list-item--active' : ''}"
        data-memory-id="${esc(m.id)}"
        aria-current="${isActive ? 'true' : 'false'}"
      >
        <span class="memories-list-item__title">${esc(m.title)}</span>
        <span class="memories-list-item__date">${esc(date)}</span>
      </button>
    `;
  }).join('');
}

function renderDetail() {
  if (!_selectedId) {
    return `
      <div class="memories-detail-empty">
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" width="40" height="40" aria-hidden="true"><rect x="4" y="4" width="16" height="16" rx="2"/><rect x="9" y="9" width="6" height="6"/><path d="M15 2v2M9 2v2M2 15h2M2 9h2M15 22v-2M9 22v-2M22 15h-2M22 9h-2"/></svg>
        <p>Select a memory to read it.</p>
      </div>
    `;
  }

  if (_contentLoading) {
    return `<div class="memories-detail-loading"><div class="loading-dots"><span></span><span></span><span></span></div></div>`;
  }

  if (!_selectedMemory) {
    return `<div class="memories-detail-error">Failed to load memory.</div>`;
  }

  const date = new Date(_selectedMemory.created_at).toLocaleString(undefined, {
    year: 'numeric', month: 'short', day: 'numeric',
    hour: '2-digit', minute: '2-digit',
  });
  const updated = _selectedMemory.updated_at !== _selectedMemory.created_at
    ? ` · Updated ${new Date(_selectedMemory.updated_at).toLocaleString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })}`
    : '';

  return `
    <div class="memories-article">
      <div class="memories-article__header">
        <div class="memories-article__meta">
          <h2 class="memories-article__title">${esc(_selectedMemory.title)}</h2>
          <span class="memories-article__date">${esc(date)}${esc(updated)}</span>
        </div>
        <button class="p-button--negative is-small" data-delete-memory="${esc(_selectedMemory.id)}" aria-label="Delete memory">Delete</button>
      </div>
      <div class="memories-article__body">
        ${renderMarkdown(_selectedMemory.content, {}, {})}
      </div>
    </div>
  `;
}

async function handlePageClick(e) {
  const memBtn = e.target.closest('[data-memory-id]');
  if (memBtn) {
    const id = memBtn.dataset.memoryId;
    await selectMemory(id);
    return;
  }

  const delBtn = e.target.closest('[data-delete-memory]');
  if (delBtn) {
    const id = delBtn.dataset.deleteMemory;
    await deleteMemory(id);
  }
}

function handleKeydown(e) {
  const modal = document.getElementById('add-memory-modal');
  if (e.key === 'Escape' && modal && !modal.hidden) closeCreateModal();
}

async function selectMemory(id) {
  if (!_projectId) return;
  _selectedId = id;
  _selectedMemory = null;
  _contentLoading = true;
  renderPage();

  try {
    _selectedMemory = await getProjectMemory(_projectId, id);
  } catch {
    _selectedMemory = null;
  }
  _contentLoading = false;
  renderPage();
}

async function deleteMemory(id) {
  if (!_projectId) return;
  const memory = _memories.find(m => m.id === id);
  const label = memory ? `"${memory.title}"` : 'this memory';
  if (!confirm(`Delete ${label}? This cannot be undone.`)) return;

  try {
    await deleteProjectMemory(_projectId, id);
    _memories = _memories.filter(m => m.id !== id);
    if (_selectedId === id) {
      _selectedId = null;
      _selectedMemory = null;
    }
    renderPage();
  } catch (err) {
    alert(`Could not delete: ${err.message}`);
  }
}

function openCreateModal() {
  const modal    = document.getElementById('add-memory-modal');
  const titleIn  = document.getElementById('memory-title-input');
  const bodyIn   = document.getElementById('memory-content-input');
  const errorEl  = document.getElementById('add-memory-error');
  if (titleIn)  titleIn.value = '';
  if (bodyIn)   bodyIn.value  = '';
  if (errorEl)  errorEl.textContent = '';
  if (modal)    modal.hidden = false;
  titleIn?.focus();
}

function closeCreateModal() {
  const modal = document.getElementById('add-memory-modal');
  if (modal) modal.hidden = true;
}

async function submitCreate() {
  if (!_projectId) return;
  const titleIn  = document.getElementById('memory-title-input');
  const bodyIn   = document.getElementById('memory-content-input');
  const errorEl  = document.getElementById('add-memory-error');
  const submitBtn = document.getElementById('add-memory-submit');

  const title   = titleIn?.value.trim() ?? '';
  const content = bodyIn?.value ?? '';

  if (!title) {
    if (errorEl) errorEl.textContent = 'Title is required.';
    return;
  }
  if (errorEl) errorEl.textContent = '';
  if (submitBtn) submitBtn.disabled = true;

  try {
    const created = await createProjectMemory(_projectId, { title, content });
    closeCreateModal();
    _memories = await listProjectMemories(_projectId);
    await selectMemory(created.id);
  } catch (err) {
    if (errorEl) errorEl.textContent = err.message;
  } finally {
    if (submitBtn) submitBtn.disabled = false;
  }
}

import { fetchProjects, createProject, fetchMyGroups } from './api.js';

let _projects       = [];
let _currentProject = null;
let _onChange       = null;
let _pendingSelectId = null;

let _container   = null;
let _toggleBtn   = null;
let _dropdown    = null;
let _listGroup   = null;
let _searchInput = null;

export function getCurrentProject() { return _currentProject; }

export function selectProjectById(id) {
  if (!id) return;
  if (_projects.length > 0) {
    const project = _projects.find(p => p.id === id);
    if (project) {
      _currentProject = { id: project.id, name: project.name };
      _updateLabel();
      _renderList('');
      if (_onChange) _onChange(_currentProject);
    }
  } else {
    _pendingSelectId = id;
  }
}

export function refreshProjectSelector() {
  _currentProject = null;
  _projects = [];
  _updateLabel();
  _renderList('');

  fetchProjects()
    .then(projects => {
      _projects = projects;
      _renderList('');
    })
    .catch(() => {});
}

function esc(s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;').replace(/</g, '&lt;')
    .replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

export function initProjectSelector(container, { onChange } = {}) {
  _container = container;
  _onChange  = onChange;

  _build();

  fetchProjects()
    .then(projects => {
      _projects = projects;
      _updateLabel();
      _renderList('');
      if (_pendingSelectId) {
        const pending = _pendingSelectId;
        _pendingSelectId = null;
        selectProjectById(pending);
      }
    })
    .catch(() => {});
}

function _build() {
  _container.innerHTML = `
    <button type="button"
      class="project-selector__toggle"
      aria-controls="project-dropdown"
      aria-expanded="false"
      aria-haspopup="listbox">
      <svg class="project-selector__folder-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
      </svg>
      <span class="project-selector__name">Select project</span>
      <svg class="project-selector__chevron" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="4 6 8 10 12 6"/></svg>
    </button>
    <div class="project-selector__dropdown"
      id="project-dropdown"
      role="listbox"
      aria-hidden="true"
      aria-label="Select project">
      <div class="project-selector__search-wrap">
        <svg class="project-selector__search-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="6.5" cy="6.5" r="4"/><line x1="10" y1="10" x2="14" y2="14"/></svg>
        <label class="u-off-screen" for="project-search">Search projects</label>
        <input type="search" id="project-search" name="project-search"
          class="project-selector__search-input" placeholder="Search projects…" autocomplete="off">
      </div>
      <div class="project-selector__list">
        <div id="project-list-group"></div>
      </div>
      <div class="project-selector__footer">
        <button type="button" class="project-selector__new-btn" id="project-create-btn">
          <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="8" y1="3" x2="8" y2="13"/><line x1="3" y1="8" x2="13" y2="8"/></svg>
          <span>New project</span>
        </button>
      </div>
    </div>
  `;

  _toggleBtn   = _container.querySelector('.project-selector__toggle');
  _dropdown    = _container.querySelector('.project-selector__dropdown');
  _listGroup   = _container.querySelector('#project-list-group');
  _searchInput = _container.querySelector('#project-search');

  _toggleBtn.addEventListener('click', () => {
    _toggleBtn.getAttribute('aria-expanded') === 'true' ? _close() : _open();
  });

  _searchInput.addEventListener('input', () => _renderList(_searchInput.value));

  _container.querySelector('#project-create-btn').addEventListener('click', () => {
    _close();
    _openCreateModal();
  });

  document.addEventListener('click', e => {
    if (_toggleBtn.getAttribute('aria-expanded') === 'true' &&
        !_container.contains(e.target) && !_dropdown.contains(e.target)) {
      _close();
    }
  });

  document.addEventListener('keydown', e => {
    if (e.key === 'Escape') _close();
  });

  _wireCreateModal();
}

function _open() {
  const rect = _toggleBtn.getBoundingClientRect();
  const w    = 584;
  let left   = rect.left;
  let top    = rect.bottom + 4;

  if (left + w > window.innerWidth - 8) {
    left = Math.max(8, window.innerWidth - w - 8);
  }

  _dropdown.style.top  = `${Math.round(top)}px`;
  _dropdown.style.left = `${Math.round(left)}px`;
  _dropdown.setAttribute('aria-hidden', 'false');
  _toggleBtn.setAttribute('aria-expanded', 'true');

  _searchInput.value = '';
  _renderList('');
  requestAnimationFrame(() => _searchInput.focus());
}

function _close() {
  _dropdown.setAttribute('aria-hidden', 'true');
  _toggleBtn.setAttribute('aria-expanded', 'false');
}

function _updateLabel() {
  const el = _toggleBtn?.querySelector('.project-selector__name');
  if (el) el.textContent = _currentProject?.name ?? 'Select project';
}

function _renderList(query) {
  if (!_listGroup) return;
  const q = query.trim().toLowerCase();
  const filtered = q
    ? _projects.filter(p =>
        p.name.toLowerCase().includes(q) ||
        (p.description ?? '').toLowerCase().includes(q) ||
        (p.group_name  ?? '').toLowerCase().includes(q))
    : _projects;

  if (filtered.length === 0) {
    _listGroup.innerHTML = q
      ? `<p class="project-selector__empty">No projects match.</p>`
      : '';
    return;
  }

  _listGroup.innerHTML = filtered.map(p => {
    const active = p.id === _currentProject?.id;
    const group  = p.group_name ?? '';
    return `
      <button type="button"
        class="project-selector__item"
        role="option"
        aria-selected="${active ? 'true' : 'false'}"
        data-id="${esc(p.id)}"
        data-name="${esc(p.name)}">
        <span class="project-item__name" title="${esc(p.name)}">${esc(p.name)}</span>
        ${group ? `<span class="project-item__group-badge" title="${esc(group)}">${esc(group)}</span>` : ''}
        <svg class="project-item__check" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 8 6 11 13 4"/></svg>
      </button>`;
  }).join('');

  _listGroup.querySelectorAll('[data-id]').forEach(btn => {
    btn.addEventListener('click', () => {
      _currentProject = { id: btn.dataset.id, name: btn.dataset.name };
      _close();
      _updateLabel();
      _renderList('');
      if (_onChange) _onChange(_currentProject);
    });
  });
}

async function _openCreateModal() {
  const nameInput   = document.getElementById('create-project-name');
  const descInput   = document.getElementById('create-project-description');
  const groupSelect = document.getElementById('create-project-group');
  const groupRow    = document.getElementById('create-project-group-row');
  const errEl       = document.getElementById('create-project-error');
  const submitBtn   = document.getElementById('modal-create-project-submit');

  nameInput.value = '';
  descInput.value = '';
  errEl.textContent = '';
  errEl.hidden = true;
  submitBtn.disabled = true;
  groupRow.hidden = false;
  groupSelect.innerHTML = '<option value="" disabled selected>Loading groups…</option>';

  document.getElementById('modal-create-project').hidden = false;
  setTimeout(() => nameInput.focus(), 50);

  let groups;
  try {
    groups = await fetchMyGroups();
  } catch {
    groups = [];
  }

  if (groups.length === 0) {
    groupSelect.innerHTML = '<option value="" disabled selected>You are not a member of any group</option>';
    groupSelect.disabled = true;
    submitBtn.disabled = true;
    return;
  }

  submitBtn.disabled = false;

  if (groups.length === 1) {
    groupSelect.innerHTML = `<option value="${esc(groups[0].id)}">${esc(groups[0].name)}</option>`;
    groupSelect.disabled = true;
  } else {
    groupSelect.innerHTML =
      `<option value="" disabled selected>Select group for project</option>` +
      groups.map(g => `<option value="${esc(g.id)}">${esc(g.name)}</option>`).join('');
    groupSelect.disabled = false;
  }
}

function _closeCreateModal() {
  document.getElementById('modal-create-project').hidden = true;
}

function _wireCreateModal() {
  document.getElementById('modal-create-project-close')
    ?.addEventListener('click', _closeCreateModal);
  document.getElementById('modal-create-project-cancel')
    ?.addEventListener('click', _closeCreateModal);
  document.getElementById('modal-create-project')
    ?.addEventListener('click', e => {
      if (e.target === e.currentTarget) _closeCreateModal();
    });

  document.getElementById('modal-create-project-submit')
    ?.addEventListener('click', async () => {
      const name  = document.getElementById('create-project-name').value.trim();
      const desc  = document.getElementById('create-project-description').value.trim();
      const gid   = document.getElementById('create-project-group')?.value;
      const errEl = document.getElementById('create-project-error');
      const btn   = document.getElementById('modal-create-project-submit');

      errEl.hidden = true;
      if (!name) {
        errEl.textContent = 'Name is required.';
        errEl.hidden = false;
        return;
      }

      btn.disabled = true;
      try {
        const project = await createProject({ name, description: desc, group_id: gid });
        _closeCreateModal();

        const projects = await fetchProjects().catch(() => _projects);
        _projects = projects;
        _currentProject = projects.find(p => p.id === project.id) ?? { id: project.id, name };
        _updateLabel();
        _renderList('');
        if (_onChange) _onChange(_currentProject);
      } catch (err) {
        errEl.textContent = err.message;
        errEl.hidden = false;
      } finally {
        btn.disabled = false;
      }
    });

  ['create-project-name', 'create-project-description'].forEach(id => {
    document.getElementById(id)
      ?.addEventListener('keydown', e => {
        if (e.key === 'Enter') document.getElementById('modal-create-project-submit').click();
      });
  });
}

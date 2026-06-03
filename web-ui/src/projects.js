import { fetchProjects, createProject, fetchMyGroups } from './api.js';

let _projects       = [];
let _currentProject = null;
let _onChange       = null;

let _container   = null;
let _toggleBtn   = null;
let _dropdown    = null;
let _listGroup   = null;
let _searchInput = null;

export function getCurrentProject() { return _currentProject; }

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
    })
    .catch(() => {});
}

function _build() {
  _container.innerHTML = `
    <div class="project-select-label">Project</div>
    <span class="project-select is-dark p-contextual-menu p-contextual-menu--left">
      <button type="button"
        class="p-button has-icon p-contextual-menu__toggle project-selector__toggle toggle is-dark"
        aria-controls="project-dropdown"
        aria-expanded="false"
        aria-pressed="false"
        aria-haspopup="true">
        <span class="project-selector__name">All projects</span>
        <i class="p-icon--chevron-down p-contextual-menu__indicator"></i>
      </button>
      <span class="p-contextual-menu__dropdown project-selector__dropdown"
        id="project-dropdown"
        aria-hidden="true"
        aria-label="Select project">
        <div class="list is-dark">
          <div class="p-search-box">
            <label class="u-off-screen" for="project-search">Search</label>
            <input type="search" id="project-search" name="project-search"
              class="p-search-box__input" placeholder="Search" autocomplete="off">
            <button type="submit" class="p-search-box__button">
              <i class="p-icon--search">Search</i>
            </button>
          </div>
          <button type="button" class="p-button has-icon p-contextual-menu__link all-projects" id="project-all-btn">
            <i class="p-icon--folder is-light"></i>
            <span>All projects</span>
          </button>
          <div class="projects" id="project-list-group"></div>
          <hr class="is-dark">
          <button type="button" class="p-button has-icon p-contextual-menu__link" id="project-create-btn">
            <i class="p-icon--plus is-light"></i>
            <span>Create project</span>
          </button>
        </div>
      </span>
    </span>
  `;

  _toggleBtn   = _container.querySelector('.p-contextual-menu__toggle');
  _dropdown    = _container.querySelector('.p-contextual-menu__dropdown');
  _listGroup   = _container.querySelector('#project-list-group');
  _searchInput = _container.querySelector('#project-search');

  _toggleBtn.addEventListener('click', () => {
    _toggleBtn.getAttribute('aria-expanded') === 'true' ? _close() : _open();
  });

  _searchInput.addEventListener('input', () => _renderList(_searchInput.value));

  _container.querySelector('#project-all-btn').addEventListener('click', () => {
    _currentProject = null;
    _close();
    _updateLabel();
    _renderList('');
    if (_onChange) _onChange(null);
  });

  _container.querySelector('#project-create-btn').addEventListener('click', () => {
    _close();
    _openCreateModal();
  });

  document.addEventListener('click', e => {
    if (_toggleBtn.getAttribute('aria-expanded') === 'true' &&
        !_container.contains(e.target)) {
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
  const w    = 284;
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
  if (el) el.textContent = _currentProject?.name ?? 'All projects';

  const allBtn = _container?.querySelector('#project-all-btn');
  if (allBtn) {
    allBtn.setAttribute('aria-current', _currentProject === null ? 'true' : 'false');
    allBtn.classList.toggle('is-active', _currentProject === null);
  }
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
      ? `<p class="p-contextual-menu__link u-text--muted u-no-margin">No projects match.</p>`
      : '';
    return;
  }

  _listGroup.innerHTML = filtered.map(p => {
    const active = p.id === _currentProject?.id;
    const desc   = p.description ?? '';
    const group  = p.group_name  ?? '';
    return `
      <div class="p-contextual-menu__group">
        <button type="button"
          class="p-contextual-menu__link link"
          aria-current="${active ? 'true' : 'false'}"
          data-id="${esc(p.id)}"
          data-name="${esc(p.name)}">
          <div title="${esc(p.name)}" class="u-truncate name">${esc(p.name)}</div>
          ${group ? `<div class="p-text--x-small u-float-right u-no-margin--bottom count">${esc(group)}</div>` : ''}
          <br>
          <div class="p-text--x-small u-no-margin--bottom u-truncate description" title="${esc(desc)}">${desc ? esc(desc) : '-'}</div>
        </button>
      </div>`;
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

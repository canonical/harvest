import { avatarColor, initials } from './utils.js';

const USERS_URL  = '/admin/users';
const GROUPS_URL = '/admin/groups';

// ── State ─────────────────────────────────────────────────────────────────────

let _users  = [];
let _groups = [];
let _editingUserId = null;

// ── Helpers ───────────────────────────────────────────────────────────────────

function esc(s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;').replace(/</g, '&lt;')
    .replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function memberCount(groupId) {
  return _users.filter(u => (u.group_ids ?? []).filter(Boolean).includes(groupId)).length;
}

// ── API ───────────────────────────────────────────────────────────────────────

async function apiFetch(url, options = {}) {
  const res = await fetch(url, {
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `Request failed (${res.status})`);
  }
  return res.json();
}

async function fetchAll() {
  const [users, groups] = await Promise.all([apiFetch(USERS_URL), apiFetch(GROUPS_URL)]);
  _users  = users;
  _groups = groups;
}

async function createGroup(name, description) {
  return apiFetch(GROUPS_URL, { method: 'POST', body: JSON.stringify({ name, description }) });
}

async function saveUser(userId, role, groupIds) {
  await apiFetch(`${USERS_URL}/${userId}/role`, { method: 'PUT', body: JSON.stringify({ role }) });
  await apiFetch(`${USERS_URL}/${userId}/groups`, { method: 'PUT', body: JSON.stringify({ group_ids: groupIds }) });
}

async function deleteGroup(groupId) {
  return apiFetch(`${GROUPS_URL}/${groupId}`, { method: 'DELETE' });
}

// ── Toast ─────────────────────────────────────────────────────────────────────

function toast(msg, type = 'positive') {
  const el = document.getElementById('admin-toast');
  el.innerHTML = `
    <div class="p-notification--${esc(type)} is-borderless">
      <div class="p-notification__content">
        <p class="p-notification__message">${esc(msg)}</p>
      </div>
    </div>`;
  el.hidden = false;
  clearTimeout(el._timer);
  el._timer = setTimeout(() => { el.hidden = true; }, 3500);
}

// ── Stats ─────────────────────────────────────────────────────────────────────

function renderStats() {
  const el = document.getElementById('admin-stats');
  el.innerHTML = `
    <span class="admin-stat-chip">
      <strong>${_users.length}</strong>&nbsp;user${_users.length !== 1 ? 's' : ''}
    </span>
    <span class="admin-stat-chip">
      <strong>${_groups.length}</strong>&nbsp;group${_groups.length !== 1 ? 's' : ''}
    </span>`;
}

// ── Tabs ──────────────────────────────────────────────────────────────────────

function switchTab(name) {
  ['users', 'groups'].forEach(t => {
    const btn   = document.getElementById(`tab-${t}`);
    const panel = document.getElementById(`panel-${t}`);
    const active = t === name;
    btn.setAttribute('aria-selected', String(active));
    btn.setAttribute('tabindex', active ? '0' : '-1');
    panel.hidden = !active;
  });
}

// ── Users table ───────────────────────────────────────────────────────────────

function renderUsers() {
  const container = document.getElementById('admin-users-container');

  if (!_users.length) {
    container.innerHTML = `
      <div class="admin-empty">
        <div class="admin-empty__icon">👤</div>
        <p>No users yet.</p>
      </div>`;
    return;
  }

  container.innerHTML = `
    <div class="admin-table-scroll">
    <table class="p-table admin-table-fixed">
      <thead>
        <tr>
          <th>User</th>
          <th class="admin-col-shrink">Role</th>
          <th>Groups</th>
          <th class="admin-col-shrink" aria-label="Actions"></th>
        </tr>
      </thead>
      <tbody>
        ${_users.map(u => {
          const gids   = (u.group_ids ?? []).filter(Boolean);
          const chips  = gids.map(gid => {
            const g = _groups.find(g => g.id === gid);
            return g ? `<span class="admin-group-chip">${esc(g.name)}</span>` : '';
          }).filter(Boolean).join('');

          const roleBadge = u.role === 'admin'
            ? `<span class="p-label p-label--positive">Admin</span>`
            : `<span class="p-label">Regular</span>`;
          const providerBadge = u.provider === 'google'
            ? `<span class="p-label p-label--information" style="margin-left:0.25rem">Google</span>`
            : '';

          return `
            <tr>
              <td>
                <div class="admin-user-cell">
                  <div class="admin-avatar" style="background:${avatarColor(u.name)}"
                    aria-hidden="true">${esc(initials(u.name))}</div>
                  <div>
                    <div style="font-weight:600;line-height:1.3">${esc(u.name)}</div>
                    <div class="u-text--muted" style="font-size:0.8125rem">${esc(u.email)}</div>
                  </div>
                </div>
              </td>
              <td style="white-space:nowrap">
                ${roleBadge}${providerBadge}
              </td>
              <td>
                <div class="admin-group-chips">
                  ${chips || '<span class="u-text--muted">—</span>'}
                </div>
              </td>
              <td class="admin-col-shrink">
                <button class="p-button--small" type="button"
                  data-edit-user="${esc(u.id)}">Edit</button>
              </td>
            </tr>`;
        }).join('')}
      </tbody>
    </table>
    </div>`;

  container.querySelectorAll('[data-edit-user]').forEach(btn => {
    btn.addEventListener('click', () => openEditUserModal(btn.dataset.editUser));
  });
}

// ── Groups table ──────────────────────────────────────────────────────────────

function renderGroups() {
  const container = document.getElementById('admin-groups-container');

  if (!_groups.length) {
    container.innerHTML = `
      <div class="admin-empty">
        <div class="admin-empty__icon">🗂</div>
        <p>No groups yet. Create one to get started.</p>
      </div>`;
    return;
  }

  container.innerHTML = `
    <div class="admin-table-scroll">
      <table class="p-table admin-table-fixed">
        <thead>
          <tr>
            <th>Name</th>
            <th>Description</th>
            <th class="admin-col-shrink">Members</th>
            <th class="admin-col-shrink" aria-label="Actions"></th>
          </tr>
        </thead>
        <tbody>
          ${_groups.map(g => `
            <tr>
              <td style="font-weight:600">${esc(g.name)}</td>
              <td class="u-text--muted">${esc(g.description || '—')}</td>
              <td class="admin-col-shrink" style="text-align:center">${memberCount(g.id)}</td>
              <td class="admin-col-shrink">
                <button class="admin-delete-btn" type="button"
                  data-delete-group="${esc(g.id)}"
                  aria-label="Delete group ${esc(g.name)}"
                  title="Delete group">
                  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"
                    stroke="currentColor" stroke-width="2" stroke-linecap="round"
                    stroke-linejoin="round" aria-hidden="true" width="16" height="16">
                    <polyline points="3 6 5 6 21 6"/>
                    <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/>
                    <path d="M10 11v6M14 11v6"/>
                    <path d="M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/>
                  </svg>
                </button>
              </td>
            </tr>`).join('')}
        </tbody>
      </table>
    </div>`;

  container.querySelectorAll('[data-delete-group]').forEach(btn => {
    btn.addEventListener('click', async () => {
      const groupId = btn.dataset.deleteGroup;
      const group   = _groups.find(g => g.id === groupId);
      if (!group) return;
      if (!confirm(`Delete group "${group.name}"? This will remove all memberships.`)) return;
      try {
        btn.disabled = true;
        await deleteGroup(groupId);
        await fetchAll();
        renderStats(); renderUsers(); renderGroups();
        toast(`Group "${group.name}" deleted.`);
      } catch (err) {
        toast(err.message, 'negative');
      }
    });
  });
}

// ── Create group modal ────────────────────────────────────────────────────────

function openCreateGroupModal() {
  document.getElementById('group-name-input').value = '';
  document.getElementById('group-desc-input').value = '';
  document.getElementById('create-group-error').textContent = '';
  document.getElementById('modal-create-group').hidden = false;
  setTimeout(() => document.getElementById('group-name-input').focus(), 50);
}

function closeCreateGroupModal() {
  document.getElementById('modal-create-group').hidden = true;
}

// ── Edit user modal ───────────────────────────────────────────────────────────

function openEditUserModal(userId) {
  const user = _users.find(u => u.id === userId);
  if (!user) return;
  _editingUserId = userId;

  const avatar = document.getElementById('modal-user-avatar');
  avatar.textContent = initials(user.name);
  avatar.style.background = avatarColor(user.name);
  document.getElementById('modal-user-name').textContent  = user.name;
  document.getElementById('modal-user-email').textContent = user.email;
  document.getElementById('edit-user-role').value = user.role;
  document.getElementById('edit-user-error').textContent  = '';

  const gids = (user.group_ids ?? []).filter(Boolean);
  const checklist = document.getElementById('edit-user-groups-list');

  if (!_groups.length) {
    checklist.innerHTML = '<p class="u-text--muted" style="margin:0">No groups exist yet.</p>';
  } else {
    checklist.innerHTML = _groups.map(g => `
      <label class="admin-checklist-item">
        <input type="checkbox" value="${esc(g.id)}" ${gids.includes(g.id) ? 'checked' : ''} />
        <span>${esc(g.name)}</span>
        ${g.description ? `<span class="u-text--muted" style="font-size:0.8125rem"> — ${esc(g.description)}</span>` : ''}
      </label>`).join('');
  }

  document.getElementById('modal-edit-user').hidden = false;
}

function closeEditUserModal() {
  document.getElementById('modal-edit-user').hidden = true;
  _editingUserId = null;
}

// ── Wire up events ────────────────────────────────────────────────────────────

function wireEvents() {
  // Tabs
  document.getElementById('tab-users').addEventListener('click', () => switchTab('users'));
  document.getElementById('tab-groups').addEventListener('click', () => switchTab('groups'));

  // Create group modal
  document.getElementById('add-group-btn').addEventListener('click', openCreateGroupModal);
  document.getElementById('modal-create-group-close').addEventListener('click', closeCreateGroupModal);
  document.getElementById('modal-create-group-cancel').addEventListener('click', closeCreateGroupModal);
  document.getElementById('modal-create-group').addEventListener('click', e => {
    if (e.target === e.currentTarget) closeCreateGroupModal();
  });

  document.getElementById('modal-create-group-submit').addEventListener('click', async () => {
    const name = document.getElementById('group-name-input').value.trim();
    const desc = document.getElementById('group-desc-input').value.trim();
    const errorEl = document.getElementById('create-group-error');
    if (!name) { errorEl.textContent = 'Name is required.'; return; }
    errorEl.textContent = '';
    const btn = document.getElementById('modal-create-group-submit');
    btn.disabled = true;
    try {
      await createGroup(name, desc);
      closeCreateGroupModal();
      await fetchAll();
      renderStats(); renderUsers(); renderGroups();
      toast('Group created.');
    } catch (err) {
      errorEl.textContent = err.message;
    } finally {
      btn.disabled = false;
    }
  });

  // Edit user modal
  document.getElementById('modal-edit-user-close').addEventListener('click', closeEditUserModal);
  document.getElementById('modal-edit-user-cancel').addEventListener('click', closeEditUserModal);
  document.getElementById('modal-edit-user').addEventListener('click', e => {
    if (e.target === e.currentTarget) closeEditUserModal();
  });

  document.getElementById('modal-edit-user-save').addEventListener('click', async () => {
    if (!_editingUserId) return;
    const role = document.getElementById('edit-user-role').value;
    const groupIds = [...document.querySelectorAll('#edit-user-groups-list input[type="checkbox"]:checked')]
      .map(cb => cb.value);
    const errorEl = document.getElementById('edit-user-error');
    const btn = document.getElementById('modal-edit-user-save');
    errorEl.textContent = '';
    btn.disabled = true;
    try {
      await saveUser(_editingUserId, role, groupIds);
      closeEditUserModal();
      await fetchAll();
      renderStats(); renderUsers(); renderGroups();
      toast('User updated.');
    } catch (err) {
      errorEl.textContent = err.message;
    } finally {
      btn.disabled = false;
    }
  });

  // Enter submits the create-group modal from either input
  ['group-name-input', 'group-desc-input'].forEach(id => {
    document.getElementById(id).addEventListener('keydown', e => {
      if (e.key === 'Enter') document.getElementById('modal-create-group-submit').click();
    });
  });

  // Escape closes any open modal
  document.addEventListener('keydown', e => {
    if (e.key === 'Escape') { closeCreateGroupModal(); closeEditUserModal(); }
  });
}

// ── Init ──────────────────────────────────────────────────────────────────────

export function initAdminPage(_container) {
  if (_container.dataset.adminInit) {
    fetchAll()
      .then(() => { renderStats(); renderUsers(); renderGroups(); })
      .catch(err => toast(err.message, 'negative'));
    return;
  }
  _container.dataset.adminInit = '1';
  wireEvents();
  fetchAll()
    .then(() => { renderStats(); renderUsers(); renderGroups(); })
    .catch(err => toast(err.message, 'negative'));
}

<template>
  <div class="admin-page">
    <div class="admin-stats">
      <span class="admin-stat-chip"><strong>{{ users.length }}</strong>&nbsp;user{{ users.length !== 1 ? 's' : '' }}</span>
      <span class="admin-stat-chip"><strong>{{ groups.length }}</strong>&nbsp;group{{ groups.length !== 1 ? 's' : '' }}</span>
    </div>

    <div v-if="toast" class="p-notification--positive" :class="toastType === 'negative' ? 'p-notification--negative' : 'p-notification--positive'" role="alert">
      <div class="p-notification__content">
        <p class="p-notification__message">{{ toast }}</p>
      </div>
    </div>

    <div class="p-tabs">
      <ul class="p-tabs__list" role="tablist">
        <li class="p-tabs__item" role="presentation">
          <button
            class="p-tabs__link"
            role="tab"
            :aria-selected="activeTab === 'users'"
            data-testid="tab-users"
            @click="activeTab = 'users'"
          >Users</button>
        </li>
        <li class="p-tabs__item" role="presentation">
          <button
            class="p-tabs__link"
            role="tab"
            :aria-selected="activeTab === 'groups'"
            data-testid="tab-groups"
            @click="activeTab = 'groups'"
          >Groups</button>
        </li>
      </ul>
    </div>

    <div v-if="activeTab === 'users'" class="admin-panel" role="tabpanel">
      <div v-if="!users.length" class="admin-empty">
        <p>No users yet.</p>
      </div>
      <div v-else class="admin-table-scroll">
        <table class="p-table">
          <thead>
            <tr>
              <th>User</th>
              <th>Role</th>
              <th>Groups</th>
              <th aria-label="Actions"></th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="u in users" :key="u.id">
              <td>
                <div class="admin-user-cell">
                  <div class="admin-avatar" :style="{ background: avatarColor(u.name) }" aria-hidden="true">{{ initials(u.name) }}</div>
                  <div>
                    <div class="admin-user-name">{{ u.name }}</div>
                    <div class="admin-user-email u-text--muted">{{ u.email }}</div>
                  </div>
                </div>
              </td>
              <td style="white-space:nowrap">
                <span class="p-label" :class="u.role === 'admin' ? 'p-label--positive' : ''">{{ u.role === 'admin' ? 'Admin' : 'Regular' }}</span>
                <span v-if="u.provider === 'google'" class="p-label p-label--information" style="margin-left:0.25rem">Google</span>
              </td>
              <td>
                <div class="admin-group-chips">
                  <template v-if="userGroupNames(u).length">
                    <span v-for="name in userGroupNames(u)" :key="name" class="p-chip admin-group-chip">{{ name }}</span>
                  </template>
                  <span v-else class="u-text--muted">—</span>
                </div>
              </td>
              <td>
                <button class="p-button p-button--small" type="button" @click="openEditUser(u)">Edit</button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <div v-if="activeTab === 'groups'" class="admin-panel" role="tabpanel">
      <div class="admin-panel-toolbar">
        <button class="p-button--positive" type="button" @click="openCreateGroup">+ Add group</button>
      </div>
      <div v-if="!groups.length" class="admin-empty">
        <p>No groups yet. Create one to get started.</p>
      </div>
      <div v-else class="admin-table-scroll">
        <table class="p-table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Description</th>
              <th>Members</th>
              <th aria-label="Actions"></th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="g in groups" :key="g.id">
              <td class="admin-group-name">{{ g.name }}</td>
              <td class="u-text--muted">{{ g.description || '—' }}</td>
              <td class="admin-member-count">{{ memberCount(g.id) }}</td>
              <td>
                <button class="p-button--negative p-button--small" type="button" :aria-label="`Delete group ${g.name}`" @click="doDeleteGroup(g)">Delete</button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <div v-if="showEditUser" class="modal" @click.self="showEditUser = false">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="showEditUser = false">✕</button>
        <h3>Edit user</h3>
        <div v-if="editingUser" class="admin-user-cell" style="margin-bottom: 1rem">
          <div class="admin-avatar" :style="{ background: avatarColor(editingUser.name) }" aria-hidden="true">{{ initials(editingUser.name) }}</div>
          <div>
            <div class="admin-user-name">{{ editingUser.name }}</div>
            <div class="admin-user-email u-text--muted">{{ editingUser.email }}</div>
          </div>
        </div>
        <div class="form-group">
          <label for="edit-user-role">Role</label>
          <select id="edit-user-role" v-model="editUserRole">
            <option value="regular">Regular</option>
            <option value="admin">Admin</option>
          </select>
        </div>
        <div class="form-group">
          <label>Groups</label>
          <div v-if="!groups.length" class="u-text--muted">No groups exist yet.</div>
          <div v-else class="admin-checklist">
            <label v-for="g in groups" :key="g.id" class="admin-checklist-item">
              <input v-model="editUserGroupIds" type="checkbox" :value="g.id" />
              <span>{{ g.name }}</span>
              <span v-if="g.description" class="u-text--muted"> — {{ g.description }}</span>
            </label>
          </div>
        </div>
        <div v-if="editUserError" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ editUserError }}</p>
          </div>
        </div>
        <div class="modal-actions">
          <button class="p-button--positive" type="button" :disabled="submitting" @click="submitEditUser">Save</button>
          <button type="button" @click="showEditUser = false">Cancel</button>
        </div>
      </div>
    </div>

    <div v-if="showCreateGroup" class="modal" @click.self="showCreateGroup = false">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="showCreateGroup = false">✕</button>
        <h3>Create group</h3>
        <div class="form-group">
          <label for="group-name-input">Group name</label>
          <input id="group-name-input" v-model="newGroupName" name="group-name" type="text" placeholder="e.g. Engineering" />
        </div>
        <div class="form-group">
          <label for="group-desc-input">Description</label>
          <input id="group-desc-input" v-model="newGroupDesc" type="text" placeholder="Optional description" />
        </div>
        <div v-if="createGroupError" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ createGroupError }}</p>
          </div>
        </div>
        <div class="modal-actions">
          <button class="p-button--positive" type="button" :disabled="!newGroupName || submitting" @click="submitCreateGroup">Create</button>
          <button type="button" @click="showCreateGroup = false">Cancel</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue';
import { avatarColor, initials } from '../lib/utils.js';
import {
  listAdminUsers,
  listAdminGroups,
  updateUserRole,
  updateUserGroups,
  createAdminGroup,
  deleteAdminGroup,
} from '../lib/api.js';

const users  = ref([]);
const groups = ref([]);

const activeTab = ref('users');

const toast     = ref('');
const toastType = ref('positive');
let _toastTimer = null;

const showEditUser     = ref(false);
const editingUser      = ref(null);
const editUserRole     = ref('regular');
const editUserGroupIds = ref([]);
const editUserError    = ref('');

const showCreateGroup  = ref(false);
const newGroupName     = ref('');
const newGroupDesc     = ref('');
const createGroupError = ref('');

const submitting = ref(false);

function userGroupNames(user) {
  const gids = (user.group_ids ?? []).filter(Boolean);
  return gids.map(id => groups.value.find(g => g.id === id)?.name).filter(Boolean);
}

function memberCount(groupId) {
  return users.value.filter(u => (u.group_ids ?? []).includes(groupId)).length;
}

function showToast(msg, type = 'positive') {
  toast.value     = msg;
  toastType.value = type;
  clearTimeout(_toastTimer);
  _toastTimer = setTimeout(() => { toast.value = ''; }, 3500);
}

async function loadAll() {
  try {
    const [u, g] = await Promise.all([listAdminUsers(), listAdminGroups()]);
    users.value  = u ?? [];
    groups.value = g ?? [];
  } catch (e) {
    showToast(e.message, 'negative');
  }
}

function openEditUser(user) {
  editingUser.value      = user;
  editUserRole.value     = user.role;
  editUserGroupIds.value = [...(user.group_ids ?? []).filter(Boolean)];
  editUserError.value    = '';
  showEditUser.value     = true;
}

async function submitEditUser() {
  if (!editingUser.value) return;
  submitting.value    = true;
  editUserError.value = '';
  try {
    await updateUserRole(editingUser.value.id, editUserRole.value);
    await updateUserGroups(editingUser.value.id, editUserGroupIds.value);
    showEditUser.value = false;
    await loadAll();
    showToast('User updated.');
  } catch (e) {
    editUserError.value = e.message;
  } finally {
    submitting.value = false;
  }
}

function openCreateGroup() {
  newGroupName.value     = '';
  newGroupDesc.value     = '';
  createGroupError.value = '';
  showCreateGroup.value  = true;
}

async function submitCreateGroup() {
  if (!newGroupName.value) return;
  submitting.value       = true;
  createGroupError.value = '';
  try {
    await createAdminGroup(newGroupName.value.trim(), newGroupDesc.value.trim());
    showCreateGroup.value = false;
    await loadAll();
    showToast('Group created.');
  } catch (e) {
    createGroupError.value = e.message;
  } finally {
    submitting.value = false;
  }
}

async function doDeleteGroup(group) {
  if (!confirm(`Delete group "${group.name}"? This will remove all memberships.`)) return;
  try {
    await deleteAdminGroup(group.id);
    await loadAll();
    showToast(`Group "${group.name}" deleted.`);
  } catch (e) {
    showToast(e.message, 'negative');
  }
}

onMounted(loadAll);
</script>

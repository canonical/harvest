<template>
  <div class="admin-page">
    <div class="admin-stats">
      <span class="admin-stat-chip"><strong>{{ users.length }}</strong>&nbsp;user{{ users.length !== 1 ? 's' : '' }}</span>
    </div>

    <div v-if="toast" :class="toastType === 'negative' ? 'p-notification--negative' : 'p-notification--positive'" role="alert">
      <div class="p-notification__content">
        <p class="p-notification__message">{{ toast }}</p>
      </div>
    </div>

    <div class="p-tabs">
      <ul class="p-tabs__list" role="tablist">
        <li v-for="tab in TABS" :key="tab.id" class="p-tabs__item" role="presentation">
          <button
            class="p-tabs__link"
            role="tab"
            :aria-selected="activeTab === tab.id"
            :data-testid="`tab-${tab.id}`"
            @click="activeTab = tab.id"
          >{{ tab.label }}</button>
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
                <button class="p-button is-dense" type="button" @click="openEditUser(u)">Edit</button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <ConversationMoves v-if="activeTab === 'moves'" @done="showToast" @failed="message => showToast(message, 'negative')" />

    <AppModal v-if="editingUser" @close="editingUser = null">
      <h2>Edit user</h2>
      <div class="admin-user-cell" style="margin-bottom: 1rem">
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
      <div v-if="editUserError" class="p-notification--negative">
        <div class="p-notification__content">
          <p class="p-notification__message">{{ editUserError }}</p>
        </div>
      </div>
      <div class="modal-actions">
        <button class="p-button--positive is-dense" type="button" :disabled="submitting" @click="submitEditUser">Save</button>
        <button type="button" @click="editingUser = null">Cancel</button>
      </div>
    </AppModal>
  </div>
</template>

<script setup>
import AppModal from '../components/ui/AppModal.vue';
import { ref, onMounted } from 'vue';
import { avatarColor, initials } from '../lib/utils.js';
import { listAdminUsers, updateUserRole } from '../lib/api.js';
import ConversationMoves from '../components/admin/ConversationMoves.vue';

const TABS = [
  { id: 'users', label: 'Users' },
  { id: 'moves', label: 'Move chats' },
];
const TOAST_MS = 3500;

const users = ref([]);
const activeTab = ref('users');
const toast = ref('');
const toastType = ref('positive');
const editingUser = ref(null);
const editUserRole = ref('regular');
const editUserError = ref('');
const submitting = ref(false);
let toastTimer = null;

function showToast(message, type = 'positive') {
  toast.value = message;
  toastType.value = type;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { toast.value = ''; }, TOAST_MS);
}

async function loadUsers() {
  try {
    users.value = (await listAdminUsers()) ?? [];
  } catch (e) {
    showToast(e.message, 'negative');
  }
}

function openEditUser(user) {
  editingUser.value = user;
  editUserRole.value = user.role;
  editUserError.value = '';
}

async function submitEditUser() {
  submitting.value = true;
  editUserError.value = '';
  try {
    await updateUserRole(editingUser.value.id, editUserRole.value);
    editingUser.value = null;
    await loadUsers();
    showToast('User updated.');
  } catch (e) {
    editUserError.value = e.message;
  } finally {
    submitting.value = false;
  }
}

onMounted(loadUsers);
</script>

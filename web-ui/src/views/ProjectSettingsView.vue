<template>
  <div class="settings-page">
    <div v-if="!projectId" class="no-project-state">
      <p>Select a project to see its settings.</p>
    </div>

    <template v-else>
      <div class="settings-header">
        <h2>Project settings</h2>
      </div>

      <section class="settings-section">
        <div class="settings-section__heading">
          <h3>Details</h3>
        </div>
        <div class="form-group">
          <label for="project-name">Name</label>
          <input id="project-name" v-model="name" type="text" :disabled="!canEdit" />
        </div>
        <div class="form-group">
          <label for="project-description">Description</label>
          <input id="project-description" v-model="description" type="text" :disabled="!canEdit" />
        </div>
        <button v-if="canEdit" class="p-button--positive is-dense" type="button" :disabled="!detailsChanged || busy" @click="saveDetails">Save</button>
      </section>

      <section class="settings-section">
        <div class="settings-section__heading">
          <h3>Members</h3>
          <p class="settings-section__description">Owners manage members. Editors chat, design, deploy and run agents. Viewers read everything in the project.</p>
        </div>
        <div v-if="canManage" class="project-members__invite">
          <input v-model="inviteEmail" type="email" placeholder="Email of a Harvest user" aria-label="Email" @keydown.enter="invite" />
          <select v-model="inviteRole" aria-label="Role">
            <option v-for="role in ROLES" :key="role" :value="role">{{ role }}</option>
          </select>
          <button class="p-button--positive is-dense" type="button" :disabled="!inviteEmail.trim() || busy" @click="invite">Share</button>
        </div>
        <table class="p-table project-members__table">
          <thead>
            <tr><th>Member</th><th>Role</th><th aria-label="Actions"></th></tr>
          </thead>
          <tbody>
            <tr v-for="member in members" :key="member.user_id">
              <td>
                <div class="admin-user-name">{{ member.name || member.email }}</div>
                <div class="u-text--muted">{{ member.email }}</div>
              </td>
              <td>
                <select v-if="canManage" :value="member.role" :aria-label="`Role of ${member.email}`" :disabled="busy" @change="changeRole(member, $event.target.value)">
                  <option v-for="role in ROLES" :key="role" :value="role">{{ role }}</option>
                </select>
                <span v-else class="p-label">{{ member.role }}</span>
              </td>
              <td>
                <button v-if="canManage" class="p-button--base is-dense" type="button" :disabled="busy" @click="remove(member)">Remove</button>
              </td>
            </tr>
          </tbody>
        </table>
      </section>

      <section v-if="canManage" class="settings-section">
        <div class="settings-section__heading">
          <h3>Delete project</h3>
          <p class="settings-section__description">Deletes the project with its chats, designs, deployments and agents.</p>
        </div>
        <button class="p-button--negative is-dense" type="button" :disabled="busy" @click="removeProject">Delete project</button>
      </section>

      <p v-if="error" class="auth-error">{{ error }}</p>
    </template>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { useProjectStore } from '../stores/project.js';
import {
  addProjectMember, deleteProject, listProjectMembers, removeProjectMember, setProjectMemberRole, updateProject,
} from '../lib/api.js';

const ROLES = ['viewer', 'editor', 'owner'];

const props = defineProps({
  projectId: { type: String, default: null },
});

const project = useProjectStore();
const members = ref([]);
const name = ref('');
const description = ref('');
const inviteEmail = ref('');
const inviteRole = ref('editor');
const busy = ref(false);
const error = ref('');

const role = computed(() => project.selectedProject?.role ?? 'viewer');
const canEdit = computed(() => role.value !== 'viewer');
const canManage = computed(() => role.value === 'owner');
const detailsChanged = computed(() =>
  name.value.trim() !== (project.selectedProject?.name ?? '') ||
  description.value !== (project.selectedProject?.description ?? '')
);

async function act(action) {
  busy.value = true;
  error.value = '';
  try {
    await action();
  } catch (e) {
    error.value = e.message;
  } finally {
    busy.value = false;
  }
}

const load = () => act(async () => {
  name.value = project.selectedProject?.name ?? '';
  description.value = project.selectedProject?.description ?? '';
  members.value = await listProjectMembers(props.projectId);
});

const saveDetails = () => act(async () => {
  await updateProject(props.projectId, { name: name.value.trim(), description: description.value });
  await project.fetchProjects();
});

const invite = () => act(async () => {
  members.value = await addProjectMember(props.projectId, inviteEmail.value.trim(), inviteRole.value);
  inviteEmail.value = '';
});

const changeRole = (member, next) => act(async () => {
  members.value = await setProjectMemberRole(props.projectId, member.user_id, next);
});

const remove = (member) => act(async () => {
  await removeProjectMember(props.projectId, member.user_id);
  members.value = members.value.filter(m => m.user_id !== member.user_id);
});

const removeProject = () => act(async () => {
  await deleteProject(props.projectId);
  project.clearProject();
  await project.fetchProjects();
});

watch(() => props.projectId, (id) => { if (id) load(); }, { immediate: true });
</script>

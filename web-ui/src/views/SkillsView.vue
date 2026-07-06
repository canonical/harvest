<template>
  <div class="skills-page">
    <div v-if="!projectId" class="no-project-state">
      <p>Select a project to manage its skills.</p>
    </div>

    <template v-else>
      <div class="p-tabs">
        <ul class="p-tabs__list" role="tablist">
          <li class="p-tabs__item" role="presentation">
            <button
              class="p-tabs__link"
              role="tab"
              :aria-selected="activeTab === 'project'"
              data-testid="tab-project"
              type="button"
              @click="activeTab = 'project'"
            >This project's skills</button>
          </li>
          <li class="p-tabs__item" role="presentation">
            <button
              class="p-tabs__link"
              role="tab"
              :aria-selected="activeTab === 'global'"
              data-testid="tab-global"
              type="button"
              @click="activeTab = 'global'"
            >Global skills</button>
          </li>
        </ul>
      </div>

      <section v-if="activeTab === 'global'" class="skills-section" role="tabpanel">
        <div class="skills-header">
          <h2>Global skills</h2>
          <button
            v-if="auth.isAdmin"
            class="p-button--positive"
            data-testid="add-global-skill"
            type="button"
            @click="openCreate('global')"
          >+ Add skill</button>
        </div>

        <div class="memories-layout">
          <div class="memories-list">
            <div v-if="globalLoading" class="memories-list-loading">
              <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
            </div>
            <p v-else-if="!globalSkills.length" class="memories-list-empty">No global skills yet.</p>
            <button
              v-for="s in globalSkills"
              :key="s.id"
              class="memories-list-item"
              :class="{ 'memories-list-item--active': s.id === selectedGlobalId }"
              :data-testid="`skill-item-${s.id}`"
              type="button"
              @click="selectGlobalSkill(s.id)"
            >
              <span class="memories-list-item__title">{{ s.name }}</span>
              <span class="memories-list-item__date">{{ s.description }}</span>
            </button>
          </div>

          <div class="memories-detail">
            <div v-if="!selectedGlobalId" class="memories-detail-empty">
              <p>Select a skill to read it.</p>
            </div>
            <div v-else-if="globalContentLoading" class="memories-detail-loading">
              <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
            </div>
            <div v-else-if="selectedGlobalSkill" class="memories-article">
              <div class="memories-article__header">
                <h2>{{ selectedGlobalSkill.name }}</h2>
                <div v-if="auth.isAdmin && !editingGlobal">
                  <button
                    class="p-button--base"
                    :data-testid="`edit-skill-${selectedGlobalSkill.id}`"
                    type="button"
                    @click="enterEditGlobal"
                  >Edit</button>
                  <button
                    class="p-button--negative"
                    :data-testid="`delete-skill-${selectedGlobalSkill.id}`"
                    type="button"
                    @click="confirmDelete('global', selectedGlobalSkill)"
                  >Delete</button>
                </div>
              </div>

              <template v-if="editingGlobal">
                <div class="form-group">
                  <label>Name</label>
                  <input v-model="editGlobalName" type="text" />
                </div>
                <div class="form-group">
                  <label>Description</label>
                  <textarea v-model="editGlobalDescription" rows="2" />
                </div>
                <div class="form-group">
                  <label>Content</label>
                  <textarea
                    v-model="editGlobalContent"
                    class="memories-textarea"
                    rows="10"
                    :data-testid="`edit-content-${selectedGlobalSkill.id}`"
                  />
                </div>
                <div v-if="editError" class="p-notification--negative">
                  <div class="p-notification__content">
                    <p class="p-notification__message">{{ editError }}</p>
                  </div>
                </div>
                <button
                  class="p-button--base"
                  :data-testid="`cancel-edit-${selectedGlobalSkill.id}`"
                  type="button"
                  @click="cancelEditGlobal"
                >Cancel</button>
                <button
                  class="p-button--positive"
                  :data-testid="`save-skill-${selectedGlobalSkill.id}`"
                  type="button"
                  :disabled="!editGlobalName || savingEdit"
                  @click="saveEditGlobal"
                >Save</button>
              </template>
              <div v-else class="memories-article__body" v-html="renderMarkdown(selectedGlobalSkill.content, {}, {})" />
            </div>
          </div>
        </div>
      </section>

      <section v-if="activeTab === 'project'" class="skills-section" role="tabpanel">
        <div class="skills-header">
          <h2>This project's skills</h2>
          <button
            class="p-button--positive"
            data-testid="add-project-skill"
            type="button"
            @click="openCreate('project')"
          >+ Add skill</button>
        </div>

        <div class="memories-layout">
          <div class="memories-list">
            <div v-if="projectLoading" class="memories-list-loading">
              <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
            </div>
            <p v-else-if="!projectSkills.length" class="memories-list-empty">No project skills yet.</p>
            <button
              v-for="s in projectSkills"
              :key="s.id"
              class="memories-list-item"
              :class="{ 'memories-list-item--active': s.id === selectedProjectId }"
              :data-testid="`skill-item-${s.id}`"
              type="button"
              @click="selectProjectSkill(s.id)"
            >
              <span class="memories-list-item__title">{{ s.name }}</span>
              <span class="memories-list-item__date">{{ s.description }}</span>
            </button>
          </div>

          <div class="memories-detail">
            <div v-if="!selectedProjectId" class="memories-detail-empty">
              <p>Select a skill to read it.</p>
            </div>
            <div v-else-if="projectContentLoading" class="memories-detail-loading">
              <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
            </div>
            <div v-else-if="selectedProjectSkill" class="memories-article">
              <div class="memories-article__header">
                <h2>{{ selectedProjectSkill.name }}</h2>
                <div v-if="!editingProject">
                  <button
                    class="p-button--base"
                    :data-testid="`edit-skill-${selectedProjectSkill.id}`"
                    type="button"
                    @click="enterEditProject"
                  >Edit</button>
                  <button
                    class="p-button--negative"
                    :data-testid="`delete-skill-${selectedProjectSkill.id}`"
                    type="button"
                    @click="confirmDelete('project', selectedProjectSkill)"
                  >Delete</button>
                </div>
              </div>

              <template v-if="editingProject">
                <div class="form-group">
                  <label>Name</label>
                  <input v-model="editProjectName" type="text" />
                </div>
                <div class="form-group">
                  <label>Description</label>
                  <textarea v-model="editProjectDescription" rows="2" />
                </div>
                <div class="form-group">
                  <label>Content</label>
                  <textarea
                    v-model="editProjectContent"
                    class="memories-textarea"
                    rows="10"
                    :data-testid="`edit-content-${selectedProjectSkill.id}`"
                  />
                </div>
                <div v-if="editError" class="p-notification--negative">
                  <div class="p-notification__content">
                    <p class="p-notification__message">{{ editError }}</p>
                  </div>
                </div>
                <button
                  class="p-button--base"
                  :data-testid="`cancel-edit-${selectedProjectSkill.id}`"
                  type="button"
                  @click="cancelEditProject"
                >Cancel</button>
                <button
                  class="p-button--positive"
                  :data-testid="`save-skill-${selectedProjectSkill.id}`"
                  type="button"
                  :disabled="!editProjectName || savingEdit"
                  @click="saveEditProject"
                >Save</button>
              </template>
              <div v-else class="memories-article__body" v-html="renderMarkdown(selectedProjectSkill.content, {}, {})" />
            </div>
          </div>
        </div>
      </section>
    </template>

    <div v-if="deletingSkill" class="modal" @click.self="deletingSkill = null">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="deletingSkill = null">✕</button>
        <h3>Delete skill</h3>
        <p>Delete <strong>{{ deletingSkill.name }}</strong>? This cannot be undone.</p>
        <div class="modal-actions">
          <button class="p-button--base" type="button" @click="deletingSkill = null">Cancel</button>
          <button
            class="p-button--negative"
            data-testid="confirm-delete"
            type="button"
            :disabled="deleting"
            @click="submitDelete"
          >Delete</button>
        </div>
      </div>
    </div>

    <div v-if="showCreate" class="modal" @click.self="showCreate = null">
      <div class="modal-content modal-content--wide">
        <button class="modal-close" type="button" @click="showCreate = null">✕</button>
        <h3>New {{ showCreate }} skill</h3>
        <div class="form-group">
          <label>Name</label>
          <input v-model="newName" type="text" data-testid="skill-name-input" placeholder="Name" />
        </div>
        <div class="form-group">
          <label>Description</label>
          <input v-model="newDescription" type="text" placeholder="Short description" />
        </div>
        <div class="form-group">
          <label>Content</label>
          <textarea v-model="newContent" class="memories-textarea" rows="8" placeholder="Write in Markdown…" />
        </div>
        <div v-if="createError" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ createError }}</p>
          </div>
        </div>
        <button
          class="p-button--positive"
          data-testid="save-create"
          type="button"
          :disabled="!newName || submitting"
          @click="submitCreate"
        >Save</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch } from 'vue';
import { renderMarkdown } from '../lib/markdown.js';
import { useAuthStore } from '../stores/auth.js';
import {
  listGlobalSkills, getGlobalSkill, createGlobalSkill, updateGlobalSkill, deleteGlobalSkill,
  listProjectSkills, getProjectSkill, createProjectSkill, updateProjectSkill, deleteProjectSkill,
} from '../lib/api.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const auth = useAuthStore();

const activeTab = ref('project');

const globalSkills        = ref([]);
const globalLoading       = ref(false);
const selectedGlobalId    = ref(null);
const selectedGlobalSkill = ref(null);
const globalContentLoading = ref(false);
const editingGlobal        = ref(false);
const editGlobalName        = ref('');
const editGlobalDescription = ref('');
const editGlobalContent     = ref('');

const projectSkills        = ref([]);
const projectLoading       = ref(false);
const selectedProjectId    = ref(null);
const selectedProjectSkill = ref(null);
const projectContentLoading = ref(false);
const editingProject        = ref(false);
const editProjectName        = ref('');
const editProjectDescription = ref('');
const editProjectContent     = ref('');

const editError  = ref('');
const savingEdit = ref(false);

const showCreate    = ref(null);
const newName        = ref('');
const newDescription = ref('');
const newContent     = ref('');
const createError    = ref('');
const submitting     = ref(false);

const deletingSkill = ref(null);
const deleting       = ref(false);

async function loadGlobalList() {
  globalLoading.value = true;
  try {
    globalSkills.value = await listGlobalSkills();
  } catch {
    globalSkills.value = [];
  }
  globalLoading.value = false;
}

async function loadProjectList() {
  if (!props.projectId) return;
  projectLoading.value = true;
  try {
    projectSkills.value = await listProjectSkills(props.projectId);
  } catch {
    projectSkills.value = [];
  }
  projectLoading.value = false;
}

async function selectGlobalSkill(id) {
  selectedGlobalId.value     = id;
  selectedGlobalSkill.value  = null;
  editingGlobal.value        = false;
  globalContentLoading.value = true;
  try {
    selectedGlobalSkill.value = await getGlobalSkill(id);
  } catch {
    selectedGlobalSkill.value = null;
  }
  globalContentLoading.value = false;
}

async function selectProjectSkill(id) {
  selectedProjectId.value     = id;
  selectedProjectSkill.value  = null;
  editingProject.value        = false;
  projectContentLoading.value = true;
  try {
    selectedProjectSkill.value = await getProjectSkill(props.projectId, id);
  } catch {
    selectedProjectSkill.value = null;
  }
  projectContentLoading.value = false;
}

function enterEditGlobal() {
  editGlobalName.value        = selectedGlobalSkill.value.name;
  editGlobalDescription.value = selectedGlobalSkill.value.description;
  editGlobalContent.value     = selectedGlobalSkill.value.content;
  editError.value             = '';
  editingGlobal.value         = true;
}

function cancelEditGlobal() {
  editingGlobal.value = false;
}

async function saveEditGlobal() {
  if (!editGlobalName.value) return;
  savingEdit.value = true;
  editError.value  = '';
  try {
    await updateGlobalSkill(selectedGlobalSkill.value.id, {
      name: editGlobalName.value,
      description: editGlobalDescription.value,
      content: editGlobalContent.value,
    });
    editingGlobal.value = false;
    await selectGlobalSkill(selectedGlobalSkill.value.id);
    await loadGlobalList();
  } catch (e) {
    editError.value = e.message;
  } finally {
    savingEdit.value = false;
  }
}

function enterEditProject() {
  editProjectName.value        = selectedProjectSkill.value.name;
  editProjectDescription.value = selectedProjectSkill.value.description;
  editProjectContent.value     = selectedProjectSkill.value.content;
  editError.value              = '';
  editingProject.value         = true;
}

function cancelEditProject() {
  editingProject.value = false;
}

async function saveEditProject() {
  if (!editProjectName.value) return;
  savingEdit.value = true;
  editError.value  = '';
  try {
    await updateProjectSkill(props.projectId, selectedProjectSkill.value.id, {
      name: editProjectName.value,
      description: editProjectDescription.value,
      content: editProjectContent.value,
    });
    editingProject.value = false;
    await selectProjectSkill(selectedProjectSkill.value.id);
    await loadProjectList();
  } catch (e) {
    editError.value = e.message;
  } finally {
    savingEdit.value = false;
  }
}

function openCreate(scope) {
  newName.value        = '';
  newDescription.value = '';
  newContent.value     = '';
  createError.value    = '';
  showCreate.value      = scope;
}

async function submitCreate() {
  if (!newName.value) return;
  submitting.value  = true;
  createError.value = '';
  try {
    if (showCreate.value === 'global') {
      const created = await createGlobalSkill({ name: newName.value, description: newDescription.value, content: newContent.value });
      showCreate.value = null;
      await loadGlobalList();
      await selectGlobalSkill(created.id);
    } else {
      const created = await createProjectSkill(props.projectId, { name: newName.value, description: newDescription.value, content: newContent.value });
      showCreate.value = null;
      await loadProjectList();
      await selectProjectSkill(created.id);
    }
  } catch (e) {
    createError.value = e.message;
  } finally {
    submitting.value = false;
  }
}

function confirmDelete(scope, skill) {
  deletingSkill.value = { scope, id: skill.id, name: skill.name };
}

async function submitDelete() {
  const target = deletingSkill.value;
  if (!target) return;
  deleting.value = true;
  try {
    if (target.scope === 'global') {
      await deleteGlobalSkill(target.id);
      globalSkills.value = globalSkills.value.filter(s => s.id !== target.id);
      if (selectedGlobalId.value === target.id) {
        selectedGlobalId.value    = null;
        selectedGlobalSkill.value = null;
      }
    } else {
      await deleteProjectSkill(props.projectId, target.id);
      projectSkills.value = projectSkills.value.filter(s => s.id !== target.id);
      if (selectedProjectId.value === target.id) {
        selectedProjectId.value    = null;
        selectedProjectSkill.value = null;
      }
    }
    deletingSkill.value = null;
  } catch {
  } finally {
    deleting.value = false;
  }
}

watch(() => props.projectId, (id) => {
  selectedProjectId.value    = null;
  selectedProjectSkill.value = null;
  projectSkills.value        = [];
  editingProject.value       = false;
  if (id) {
    loadGlobalList();
    loadProjectList();
  }
}, { immediate: true });
</script>

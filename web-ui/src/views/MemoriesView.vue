<template>
  <div class="memories-page">
    <div v-if="!projectId" class="no-project-state">
      <p>Select a project to manage its memories.</p>
    </div>

    <template v-else>
      <div class="memories-header">
        <h2>Memories</h2>
        <button class="p-button--positive add-memory-btn" data-testid="add-memory" type="button" @click="openCreate">+ Add memory</button>
      </div>

      <div class="memories-layout">
        <div class="memories-list">
          <div v-if="loading" class="memories-list-loading">
            <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
          </div>
          <p v-else-if="!memories.length" class="memories-list-empty">
            No memories yet. Create one to get started.
          </p>
          <button
            v-for="m in memories"
            :key="m.id"
            class="memories-list-item memory-item"
            :class="{ 'memories-list-item--active': m.id === selectedId }"
            @click="selectMemory(m.id)"
          >
            <span class="memories-list-item__title">{{ m.title }}</span>
            <span class="memories-list-item__date">{{ formatDate(m.created_at) }}</span>
          </button>
        </div>

        <div class="memories-detail">
          <div v-if="!selectedId" class="memories-detail-empty">
            <p>Select a memory to read it.</p>
          </div>
          <div v-else-if="contentLoading" class="memories-detail-loading">
            <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
          </div>
          <div v-else-if="selectedMemory" class="memories-article">
            <div class="memories-article__header">
              <h2>{{ selectedMemory.title }}</h2>
              <button
                class="p-button--negative delete-memory-btn"
                type="button"
                @click="confirmDeleteMemory(selectedMemory)"
              >Delete</button>
            </div>
            <div class="memories-article__body" v-html="renderedContent" />
          </div>
          <div v-else class="memories-detail-error">Failed to load memory.</div>
        </div>
      </div>
    </template>

    <div v-if="deletingMemory" class="modal" @click.self="deletingMemory = null">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="deletingMemory = null">✕</button>
        <h3>Delete memory</h3>
        <p>Delete <strong>{{ deletingMemory.title }}</strong>? This cannot be undone.</p>
        <div class="modal-actions">
          <button class="p-button--base" type="button" @click="deletingMemory = null">Cancel</button>
          <button class="p-button--negative" type="button" :disabled="submitting" @click="submitDeleteMemory">Delete</button>
        </div>
      </div>
    </div>

    <div v-if="showCreate" class="modal" @click.self="showCreate = false">
      <div class="modal-content">
        <button class="modal-close" @click="showCreate = false">✕</button>
        <h3>New memory</h3>
        <div class="form-group">
          <label>Title</label>
          <input v-model="newTitle" type="text" placeholder="Title" />
        </div>
        <div class="form-group">
          <label>Content</label>
          <textarea v-model="newContent" rows="8" placeholder="Write in Markdown…" />
        </div>
        <div v-if="createError" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ createError }}</p>
          </div>
        </div>
        <button class="p-button--positive" type="button" :disabled="!newTitle || submitting" @click="submitCreate">Save</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { renderMarkdown } from '../lib/markdown.js';
import {
  listProjectMemories,
  getProjectMemory,
  createProjectMemory,
  deleteProjectMemory,
} from '../lib/api.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const memories       = ref([]);
const loading        = ref(false);
const selectedId     = ref(null);
const selectedMemory = ref(null);
const contentLoading = ref(false);
const showCreate     = ref(false);
const deletingMemory = ref(null);
const newTitle       = ref('');
const newContent     = ref('');
const createError    = ref('');
const submitting     = ref(false);

const renderedContent = computed(() =>
  selectedMemory.value ? renderMarkdown(selectedMemory.value.content, {}, {}) : ''
);

function formatDate(iso) {
  return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

async function loadList() {
  if (!props.projectId) return;
  loading.value = true;
  try {
    memories.value = await listProjectMemories(props.projectId);
  } catch {
    memories.value = [];
  }
  loading.value = false;
}

async function selectMemory(id) {
  selectedId.value     = id;
  selectedMemory.value = null;
  contentLoading.value = true;
  try {
    selectedMemory.value = await getProjectMemory(props.projectId, id);
  } catch {
    selectedMemory.value = null;
  }
  contentLoading.value = false;
}

function confirmDeleteMemory(memory) {
  deletingMemory.value = memory;
}

async function submitDeleteMemory() {
  const memory = deletingMemory.value;
  if (!memory) return;
  submitting.value = true;
  try {
    await deleteProjectMemory(props.projectId, memory.id);
    memories.value = memories.value.filter(m => m.id !== memory.id);
    if (selectedId.value === memory.id) {
      selectedId.value     = null;
      selectedMemory.value = null;
    }
    deletingMemory.value = null;
  } catch {
  } finally {
    submitting.value = false;
  }
}

function openCreate() {
  newTitle.value   = '';
  newContent.value = '';
  createError.value = '';
  showCreate.value = true;
}

async function submitCreate() {
  if (!newTitle.value) return;
  submitting.value = true;
  createError.value = '';
  try {
    const created = await createProjectMemory(props.projectId, { title: newTitle.value, content: newContent.value });
    showCreate.value = false;
    memories.value = await listProjectMemories(props.projectId);
    await selectMemory(created.id);
  } catch (e) {
    createError.value = e.message;
  } finally {
    submitting.value = false;
  }
}

watch(() => props.projectId, (id) => {
  selectedId.value     = null;
  selectedMemory.value = null;
  memories.value       = [];
  if (id) loadList();
}, { immediate: true });
</script>

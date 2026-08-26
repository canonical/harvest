<template>
  <div class="templates-page">
    <div class="templates-header">
      <div class="templates-header__title">
        <h2>Product templates</h2>
        <span v-if="templates.length" class="templates-header__count">{{ templates.length }}</span>
      </div>
      <button class="p-button--positive is-dense" type="button" data-testid="upload-template-btn" @click="openUploadModal">Upload</button>
    </div>

    <div class="templates-layout">
      <div class="templates-list">
        <div v-if="loading" class="templates-list-loading">
          <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
        </div>
        <p v-else-if="!templates.length" class="templates-list-empty">
          No templates yet. Upload a <code>.harvest</code> file to create one.
        </p>
        <button
          v-for="t in templates"
          :key="t.id"
          class="templates-list-item"
          :class="{ 'templates-list-item--active': t.id === selectedId }"
          :data-testid="`template-item-${t.id}`"
          @click="selectTemplate(t.id)"
        >
          <span class="templates-list-item__title">{{ t.name }}</span>
          <span class="templates-list-item__date">{{ formatDate(t.created_at) }}</span>
        </button>
      </div>

      <div class="templates-detail">
        <div v-if="!selectedId" class="templates-detail-empty">
          <p v-if="templates.length">Select a template on the left to view its contents.</p>
          <p v-else>Upload a <code>.harvest</code> archive to get started.</p>
        </div>
        <div v-else-if="detailLoading" class="templates-detail-loading">
          <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
        </div>
        <template v-else-if="selectedDetail">
          <div class="templates-article">
            <div class="templates-article__header">
              <div class="templates-article__meta">
                <h3 class="templates-article__title">{{ selectedDetail.name }}</h3>
                <div class="templates-article__subline">
                  <span class="templates-article__date">{{ createdByLabel(selectedDetail.created_by) }} · {{ formatDate(selectedDetail.created_at) }}</span>
                </div>
              </div>
              <div class="templates-article__actions">
                <button
                  class="console-icon-btn console-icon-btn--danger"
                  type="button"
                  data-testid="delete-template-btn"
                  title="Delete template"
                  aria-label="Delete template"
                  @click="confirmDelete"
                >
                  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/></svg>
                </button>
              </div>
            </div>

            <section v-if="parsedContent.skills.length" class="templates-section">
              <h4 class="templates-section__title">Skills</h4>
              <div class="templates-skills">
                <div
                  v-for="s in parsedContent.skills"
                  :key="s.name"
                  class="templates-skill-card"
                  :class="{ 'templates-skill-card--active': selectedSkillName === s.name }"
                  @click="selectedSkillName = selectedSkillName === s.name ? null : s.name"
                >
                  <div class="templates-skill-card__header">
                    <span class="templates-skill-card__name">{{ s.name }}</span>
                  </div>
                  <p class="templates-skill-card__desc">{{ s.description }}</p>
                  <div v-if="selectedSkillName === s.name" class="templates-skill-card__body doc-body" v-html="renderSkillContent(s.content)" />
                </div>
              </div>
            </section>

            <section v-if="parsedContent.artifacts.length" class="templates-section">
              <h4 class="templates-section__title">Artifacts</h4>
              <div class="templates-artifacts">
                <div
                  v-for="a in parsedContent.artifacts"
                  :key="a.name"
                  class="templates-artifact-card"
                >
                  <div class="templates-artifact-card__header">
                    <span class="templates-artifact-card__name">{{ a.name }}</span>
                    <span class="artifact-kind-badge" :class="kindBadgeClass(a.kind)">{{ kindLabel(a.kind) }}</span>
                  </div>
                </div>
              </div>
            </section>

            <div v-if="!parsedContent.skills.length && !parsedContent.artifacts.length" class="templates-section-empty">
              This template has no skills or artifacts.
            </div>
          </div>
        </template>
        <div v-else class="templates-detail-error">Failed to load template.</div>
      </div>
    </div>

    <div v-if="uploadModalOpen" class="modal" @click.self="closeUploadModal">
      <div class="modal-content" data-testid="upload-template-modal">
        <button class="modal-close" type="button" @click="closeUploadModal">✕</button>
        <h3>Upload product template</h3>
        <p class="modal-lede">Upload a <code>.harvest</code> file — a zip archive containing a <code>skills/</code> directory with <code>.md</code> files (with YAML frontmatter), an <code>artifacts/</code> directory with example Terraform/Terragrunt/Bash files, and an <code>index.json</code> manifest listing them.</p>
        <label
          class="upload-dropzone"
          :class="{ 'upload-dropzone--active': dragActive }"
          data-testid="template-dropzone"
          @dragover.prevent="dragActive = true"
          @dragleave.prevent="dragActive = false"
          @drop.prevent="onDrop"
        >
          <input type="file" class="upload-dropzone__input" data-testid="template-file-input" accept=".harvest" @change="onFileChange" />
          <template v-if="!uploadFile">
            <span class="upload-dropzone__hint">Drop a .harvest file here, or click to select one</span>
          </template>
          <template v-else>
            <span class="upload-dropzone__file">{{ uploadFile.name }}</span>
          </template>
        </label>
        <div v-if="uploadError" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ uploadError }}</p>
          </div>
        </div>
        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" @click="closeUploadModal">Cancel</button>
          <button class="p-button--positive is-dense" type="button" data-testid="submit-upload-template" :disabled="uploadSubmitting || !uploadFile" @click="submitUpload">
            {{ uploadSubmitting ? 'Uploading…' : 'Upload' }}
          </button>
        </div>
      </div>
    </div>

    <div v-if="deleteModalOpen" class="modal" @click.self="deleteModalOpen = false">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="deleteModalOpen = false">✕</button>
        <h3>Delete template</h3>
        <p>Delete <strong>{{ selectedDetail?.name }}</strong>? This cannot be undone.</p>
        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" @click="deleteModalOpen = false">Cancel</button>
          <button class="p-button--negative is-dense" type="button" data-testid="confirm-delete-template" :disabled="deleting" @click="submitDelete">
            {{ deleting ? 'Deleting…' : 'Delete' }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { renderMarkdown } from '../lib/markdown.js';
import { listTemplates, getTemplate, deleteTemplate, uploadTemplate } from '../lib/api.js';

const templates       = ref([]);
const loading         = ref(false);
const selectedId      = ref(null);
const selectedDetail  = ref(null);
const detailLoading   = ref(false);
const selectedSkillName = ref(null);

const uploadModalOpen  = ref(false);
const uploadFile       = ref(null);
const uploadSubmitting = ref(false);
const uploadError      = ref(null);
const dragActive       = ref(false);

const deleteModalOpen  = ref(false);
const deleting         = ref(false);

const parsedContent = computed(() => {
  if (!selectedDetail.value?.content) return { skills: [], artifacts: [] };
  try {
    const parsed = JSON.parse(selectedDetail.value.content);
    return {
      skills: parsed.skills ?? [],
      artifacts: parsed.artifacts ?? [],
    };
  } catch {
    return { skills: [], artifacts: [] };
  }
});

function kindLabel(kind) {
  if (kind === 'pdf') return 'PDF';
  if (kind === 'terraform') return 'Terraform';
  if (kind === 'terragrunt') return 'Terragrunt';
  if (kind === 'bash') return 'Bash';
  return 'Markdown';
}

function kindBadgeClass(kind) {
  if (kind === 'pdf') return 'artifact-kind-badge--pdf';
  if (kind === 'terraform' || kind === 'terragrunt') return 'artifact-kind-badge--terraform';
  return 'artifact-kind-badge--markdown';
}

function renderSkillContent(content) {
  return renderMarkdown(content, {}, {});
}

function formatDate(iso) {
  return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

function createdByLabel(createdBy) {
  return createdBy === 'assistant' ? 'Generated by the assistant' : `Created by ${createdBy}`;
}

async function loadList() {
  loading.value = true;
  try {
    templates.value = await listTemplates();
  } catch {
    templates.value = [];
  }
  loading.value = false;
}

async function loadDetail(id) {
  selectedId.value = id;
  selectedDetail.value = null;
  selectedSkillName.value = null;
  detailLoading.value = true;
  try {
    selectedDetail.value = await getTemplate(id);
  } catch {
    selectedDetail.value = null;
  }
  detailLoading.value = false;
}

function selectTemplate(id) {
  loadDetail(id);
}

function openUploadModal() {
  uploadFile.value = null;
  uploadError.value = null;
  dragActive.value = false;
  uploadModalOpen.value = true;
}

function closeUploadModal() {
  uploadModalOpen.value = false;
}

function onFileChange(e) {
  const f = e.target?.files?.[0];
  if (f) setUploadFile(f);
}

function onDrop(e) {
  dragActive.value = false;
  const f = e.dataTransfer?.files?.[0];
  if (f) setUploadFile(f);
}

function setUploadFile(file) {
  uploadError.value = null;
  if (!file.name.toLowerCase().endsWith('.harvest')) {
    uploadFile.value = null;
    uploadError.value = `"${file.name}" is not a .harvest file. Only .harvest archives are supported.`;
    return;
  }
  uploadFile.value = file;
}

async function submitUpload() {
  if (!uploadFile.value) return;
  uploadSubmitting.value = true;
  uploadError.value = null;
  try {
    await uploadTemplate(uploadFile.value);
    uploadModalOpen.value = false;
    await loadList();
  } catch (e) {
    uploadError.value = e.message || 'Failed to upload template';
  } finally {
    uploadSubmitting.value = false;
  }
}

function confirmDelete() {
  deleteModalOpen.value = true;
}

async function submitDelete() {
  if (!selectedId.value) return;
  deleting.value = true;
  try {
    await deleteTemplate(selectedId.value);
    templates.value = templates.value.filter(t => t.id !== selectedId.value);
    selectedId.value = null;
    selectedDetail.value = null;
    deleteModalOpen.value = false;
  } catch {
  } finally {
    deleting.value = false;
  }
}

watch(() => [], () => loadList(), { immediate: true });
</script>
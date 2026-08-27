<template>
  <div class="design-panel">
    <div v-if="error" class="p-notification--negative">
      <div class="p-notification__content">
        <p class="p-notification__message">{{ error }}</p>
      </div>
    </div>

    <template v-if="deployment.design_doc">
      <header class="design-panel__header" data-testid="design-doc-header">
        <div class="design-panel__heading">
          <div class="design-panel__title-row">
            <span
              v-if="deployment.template"
              class="design-panel__template-chip"
              data-testid="design-template-chip"
            >{{ deployment.template.name }}</span>
            <span class="design-panel__doc-name">{{ designDocTitle }}</span>
            <span v-if="docMeta" class="design-panel__doc-meta">{{ docMeta }}</span>
          </div>
        </div>
        <div class="design-panel__actions" v-if="!editing">
          <a
            class="p-button--positive artifact-download-btn is-dense"
            :href="artifactDownloadUrl(deployment.design_doc.id)"
            download
            data-testid="download-design-btn"
          >
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
            Download
          </a>
          <button
            class="p-button--positive is-dense"
            data-testid="edit-design-btn"
            type="button"
            :disabled="proposing"
            @click="startEdit"
          >Edit</button>
          <button
            class="p-button--brand is-dense"
            data-testid="propose-toggle-btn"
            type="button"
            :disabled="proposing"
            @click="proposeOpen = true"
          >Propose a change</button>
          <BusyStatus v-if="busyLabel" :text="busyLabel" />
        </div>
      </header>

      <div class="design-panel__content">
        <div
          v-if="editing"
          ref="editorContainerRef"
          class="design-panel__editor-container"
          data-testid="design-editor"
        />

        <div
          v-else
          class="design-panel__preview doc-body"
          data-testid="design-preview"
          v-html="renderedDesign"
        />
      </div>

      <div v-if="editing" class="design-panel__edit-bar" data-testid="design-edit-bar">
        <button class="p-button--base is-dense" type="button" data-testid="cancel-edit-btn" @click="cancelEdit">Cancel</button>
        <button class="p-button--positive is-dense" type="button" data-testid="save-design-btn" :disabled="saving" @click="saveEdit">
          {{ saving ? 'Saving…' : 'Save' }}
        </button>
      </div>
    </template>

    <div v-if="proposeOpen" class="modal" @click.self="closePropose">
      <div class="modal-content" data-testid="design-prompt-panel">
        <button class="modal-close" type="button" @click="closePropose">✕</button>
        <h3>Propose a change</h3>
        <p class="modal-lede">Describe the change you'd like to propose for this design document.</p>
        <div class="form-group">
          <label for="design-prompt">Change description</label>
          <textarea
            id="design-prompt"
            v-model="promptText"
            rows="6"
            data-testid="design-prompt"
            placeholder="Describe what you'd like to change"
          />
        </div>
        <div v-if="error" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ error }}</p>
          </div>
        </div>
        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" @click="closePropose">Cancel</button>
          <button
            class="p-button--positive is-dense"
            type="button"
            data-testid="propose-design-btn"
            :disabled="!promptText.trim() || proposing"
            @click="propose"
          >{{ proposing ? 'Proposing…' : 'Propose' }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onBeforeUnmount, nextTick } from 'vue';
import { renderMarkdown } from '../../lib/markdown.js';
import { getArtifact, updateArtifact, proposeArtifactChange, artifactDownloadUrl } from '../../lib/api.js';
import BusyStatus from './BusyStatus.vue';

const props = defineProps({
  projectId:  { type: String, required: true },
  deployment: { type: Object, required: true },
});
const emit = defineEmits(['refresh']);

const designContent = ref('');
const editing       = ref(false);
const saving        = ref(false);
const proposing     = ref(false);
const proposeOpen   = ref(false);
const promptText    = ref('');
const error         = ref(null);

const editorContainerRef = ref(null);
let editor               = null;
let monacoApi            = null;
let originalContent      = '';

const renderedDesign = computed(() => designContent.value ? renderMarkdown(designContent.value, {}, {}) : '');

const designDocTitle = computed(() => props.deployment.design_doc?.title ?? 'Design');

const docMeta = computed(() => {
  const parts = [];
  const by = props.deployment.created_by;
  if (by) parts.push(by === 'assistant' ? 'Generated by the assistant' : `Created by ${by}`);
  const at = props.deployment.created_at;
  if (at) parts.push(formatDate(at));
  return parts.join(' · ');
});

const busyLabel = computed(() => {
  if (saving.value)    return 'Saving design…';
  if (proposing.value) return 'Proposing a change…';
  return null;
});

function formatDate(iso) {
  return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

async function loadDesignContent() {
  if (!props.deployment.design_doc) {
    designContent.value = '';
    return;
  }
  try {
    const artifact = await getArtifact(props.deployment.design_doc.id);
    designContent.value = artifact.content || '';
  } catch {
    designContent.value = '';
  }
}

async function startEdit() {
  if (editing.value) {
    cancelEdit();
    return;
  }
  originalContent = designContent.value;
  editing.value = true;
  proposeOpen.value = false;
  await nextTick();
  await mountEditor();
}

function closePropose() {
  proposeOpen.value = false;
  promptText.value = '';
  error.value = null;
}

async function mountEditor() {
  if (editor) {
    editor.dispose();
    editor = null;
  }
  if (!editorContainerRef.value) return;
  if (!monacoApi) {
    monacoApi = await import('monaco-editor');
  }
  editor = monacoApi.editor.create(editorContainerRef.value, {
    value: originalContent,
    language: 'markdown',
    automaticLayout: true,
    minimap: { enabled: false },
    fontSize: 13,
    lineNumbers: 'on',
    wordWrap: 'on',
    scrollBeyondLastLine: false,
  });
}

function cancelEdit() {
  editing.value = false;
  if (editor) {
    editor.dispose();
    editor = null;
  }
}

async function saveEdit() {
  if (!editor || saving.value) return;
  const newContent = editor.getValue();
  saving.value = true;
  error.value = null;
  try {
    await updateArtifact(props.deployment.design_doc.id, {
      title: props.deployment.design_doc.title ?? 'Design',
      kind: 'markdown',
      content: newContent,
    });
    designContent.value = newContent;
    originalContent = newContent;
    if (editor) {
      editor.dispose();
      editor = null;
    }
    editing.value = false;
    emit('refresh');
  } catch (e) {
    error.value = e.message || 'Failed to save';
  } finally {
    saving.value = false;
  }
}

async function propose() {
  if (!promptText.value.trim()) return;
  proposing.value = true;
  error.value = null;
  try {
    await proposeArtifactChange(props.projectId, props.deployment.id, {
      artifact_id: props.deployment.design_doc.id,
      source: 'prompt',
      explanation: promptText.value.trim(),
      current_content: designContent.value,
      proposed_content: promptText.value.trim(),
    });
    promptText.value = '';
    proposeOpen.value = false;
  } catch (e) {
    error.value = e.message || 'Failed to propose change';
  } finally {
    proposing.value = false;
  }
}

onBeforeUnmount(() => {
  if (editor) {
    editor.dispose();
    editor = null;
  }
});

watch(() => props.deployment.design_doc?.id, loadDesignContent, { immediate: true });
</script>

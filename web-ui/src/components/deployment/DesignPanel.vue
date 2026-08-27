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
              class="p-chip"
              data-testid="design-template-chip"
            >{{ deployment.template.name }}</span>
            <h3 class="p-heading--4 design-panel__doc-name">{{ designDocTitle }}</h3>
          </div>
          <p v-if="docMeta" class="p-text--small u-text--muted design-panel__doc-meta">{{ docMeta }}</p>
        </div>
        <div class="design-panel__actions" v-if="!editing && !proposalPhase">
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
            @click="openPropose"
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
          v-else-if="proposalPhase === 'generating' && !proposedContent"
          class="design-panel__preview doc-body design-panel__proposal-generating"
          data-testid="design-proposal-generating"
        >
          <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
          <p class="u-text--muted">Generating a proposed change…</p>
        </div>

        <div
          v-else-if="proposalPhase === 'generating'"
          class="design-panel__preview doc-body"
          data-testid="design-proposal-streaming"
          v-html="renderedProposalStream"
        />

        <div
          v-else-if="proposalPhase === 'reviewing'"
          class="design-panel__preview doc-body"
          data-testid="design-proposal-diff"
        >
          <DiffView :before="{ [designDocTitle]: designContent }" :after="{ [designDocTitle]: proposedContent }" />
        </div>

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

      <div v-if="proposalPhase === 'reviewing'" class="design-panel__edit-bar" data-testid="design-proposal-bar">
        <button class="p-button--positive is-dense" type="button" data-testid="apply-proposal-btn" :disabled="proposing" @click="applyProposal">
          {{ proposing ? 'Applying…' : 'Apply' }}
        </button>
        <button class="p-button--base is-dense" type="button" data-testid="modify-proposal-btn" @click="modifyProposal">Modify</button>
        <button class="p-button--negative is-dense" type="button" data-testid="discard-proposal-btn" @click="discardProposal">Discard</button>
      </div>
    </template>

    <div v-if="proposeOpen" class="modal" @click.self="closePropose">
      <div class="modal-content modal-content--xwide" data-testid="design-prompt-panel">
        <button class="modal-close" type="button" @click="closePropose">✕</button>
        <h3>Propose a change</h3>
        <p class="modal-lede">Describe the change you'd like to propose for this design document.</p>

        <div class="design-panel__propose-grid">
          <div class="form-group">
            <label for="design-prompt">Change description</label>
            <textarea
              id="design-prompt"
              v-model="promptText"
              rows="10"
              data-testid="design-prompt"
              placeholder="Describe what you'd like to change"
            />
          </div>

          <div class="form-group">
            <label>Context artifacts</label>
            <p class="p-text--small u-text--muted">
              Artifacts already used to generate this design are pre-selected and locked.
              Select additional artifacts to add as context for this change.
            </p>
            <div v-if="artifactsLoading" class="design-setup__loading">
              <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
            </div>
            <div v-else-if="!userProvidedArtifacts.length" class="design-setup__empty">
              <p class="u-text--muted">This project has no user-provided artifacts yet.</p>
            </div>
            <ul v-else class="p-list--divided design-setup__list design-setup__list--modal">
              <li v-for="a in userProvidedArtifacts" :key="a.id" class="p-list__item design-setup__item">
                <div class="p-checkbox design-setup__item-checkbox">
                  <input
                    type="checkbox"
                    class="p-checkbox__input"
                    :id="`propose-artifact-checkbox-${a.id}`"
                    :value="a.id"
                    v-model="selectedArtifactIds"
                    :data-testid="`propose-artifact-checkbox-${a.id}`"
                    :disabled="usedArtifactIds.has(a.id)"
                  />
                  <label class="p-checkbox__label" :for="`propose-artifact-checkbox-${a.id}`"></label>
                </div>
                <label class="design-setup__item-title" :for="`propose-artifact-checkbox-${a.id}`">{{ a.title }}</label>
                <span class="artifact-kind-badge" :class="kindBadgeClass(a.kind)">{{ kindLabel(a.kind) }}</span>
              </li>
            </ul>
          </div>
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
import {
  getArtifact, updateArtifact, proposeDesignChangeStream, artifactDownloadUrl,
  listProjectArtifacts, linkContextArtifact,
} from '../../lib/api.js';
import BusyStatus from './BusyStatus.vue';
import DiffView from './DiffView.vue';

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

const artifacts           = ref([]);
const artifactsLoading    = ref(false);
const selectedArtifactIds = ref([]);

const proposalPhase      = ref(null);
const proposedContent    = ref('');
const proposalExplanation = ref('');
const proposalArtifactIds = ref([]);

const usedArtifactIds = computed(() => new Set((props.deployment.context_artifacts ?? []).map(a => a.id)));

const nonUserProvidedIds = computed(() => {
  const ids = new Set();
  if (props.deployment.design_doc)       ids.add(props.deployment.design_doc.id);
  if (props.deployment.guide)            ids.add(props.deployment.guide.id);
  if (props.deployment.terraform_bundle) ids.add(props.deployment.terraform_bundle.id);
  return ids;
});

const userProvidedArtifacts = computed(() => artifacts.value.filter(a => !nonUserProvidedIds.value.has(a.id)));

const editorContainerRef = ref(null);
let editor               = null;
let monacoApi            = null;
let originalContent      = '';

const renderedDesign = computed(() => designContent.value ? renderMarkdown(designContent.value, {}, {}) : '');

const renderedProposalStream = computed(() => proposedContent.value ? renderMarkdown(proposedContent.value, {}, {}) : '');

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

function stripMarkdownFence(text) {
  const trimmed = text.trim();
  if (!trimmed.startsWith('```')) return trimmed;
  const withoutOpen = trimmed.replace(/^```[a-zA-Z]*\n?/, '');
  return withoutOpen.replace(/```\s*$/, '').trim();
}

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
  if (kind === 'bash') return 'artifact-kind-badge--bash';
  return 'artifact-kind-badge--markdown';
}

async function loadArtifacts() {
  artifactsLoading.value = true;
  try {
    artifacts.value = await listProjectArtifacts(props.projectId);
  } catch {
    artifacts.value = [];
  }
  artifactsLoading.value = false;
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

function openPropose() {
  proposeOpen.value = true;
  promptText.value = '';
  selectedArtifactIds.value = [...usedArtifactIds.value];
  loadArtifacts();
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
    const newArtifactIds = selectedArtifactIds.value.filter(id => !usedArtifactIds.value.has(id));
    for (const artifactId of newArtifactIds) {
      await linkContextArtifact(props.projectId, props.deployment.id, { artifact_id: artifactId });
    }
    proposalExplanation.value = promptText.value.trim();
    proposalArtifactIds.value = [...selectedArtifactIds.value];
    proposeOpen.value = false;
    proposalPhase.value = 'generating';
    proposedContent.value = '';
    let streamError = null;
    let finalAnswer = '';
    await proposeDesignChangeStream(props.projectId, props.deployment.id, {
      explanation: proposalExplanation.value,
      artifact_ids: proposalArtifactIds.value,
    }, (event) => {
      if (!event) return;
      if (event.type === 'text_delta') {
        proposedContent.value += event.text || '';
      } else if (event.type === 'done') {
        finalAnswer = event.answer || '';
      } else if (event.type === 'error') {
        streamError = event.message || 'Failed to propose change';
      }
    });
    if (finalAnswer.trim()) {
      proposedContent.value = stripMarkdownFence(finalAnswer.trim());
    }
    if (streamError || !proposedContent.value.trim()) {
      error.value = streamError || 'Failed to propose change';
      proposalPhase.value = null;
    } else {
      proposalPhase.value = 'reviewing';
    }
  } catch (e) {
    error.value = e.message || 'Failed to propose change';
    proposalPhase.value = null;
  } finally {
    proposing.value = false;
  }
}

async function applyProposal() {
  proposing.value = true;
  error.value = null;
  try {
    await updateArtifact(props.deployment.design_doc.id, {
      title: props.deployment.design_doc.title ?? 'Design',
      kind: 'markdown',
      content: proposedContent.value,
    });
    designContent.value = proposedContent.value;
    originalContent = proposedContent.value;
    proposalPhase.value = null;
    proposedContent.value = '';
    emit('refresh');
  } catch (e) {
    error.value = e.message || 'Failed to apply change';
  } finally {
    proposing.value = false;
  }
}

function discardProposal() {
  proposalPhase.value = null;
  proposedContent.value = '';
  error.value = null;
}

function modifyProposal() {
  proposalPhase.value = null;
  proposedContent.value = '';
  promptText.value = proposalExplanation.value;
  selectedArtifactIds.value = [...proposalArtifactIds.value];
  proposeOpen.value = true;
  loadArtifacts();
}

onBeforeUnmount(() => {
  if (editor) {
    editor.dispose();
    editor = null;
  }
});

watch(() => props.deployment.design_doc?.id, loadDesignContent, { immediate: true });
</script>

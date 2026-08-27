<template>
  <div class="artifact-editor" data-testid="artifact-editor">
    <div v-if="!artifactId" class="artifact-editor__empty" data-testid="artifact-editor-empty">
      <p>Select a node in the DAG to view its artifact.</p>
    </div>

    <div v-else-if="loading" class="artifact-editor__loading" data-testid="artifact-editor-loading">
      <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
    </div>

    <div v-else-if="artifact" class="artifact-editor__content">
      <div class="artifact-editor__header">
        <div class="artifact-editor__meta">
          <h3 class="artifact-editor__title">{{ artifact.title }}</h3>
          <span class="artifact-kind-badge" :class="kindBadgeClass(artifact.kind)">{{ kindLabel(artifact.kind) }}</span>
        </div>
        <div class="artifact-editor__actions">
          <button
            v-if="dirty"
            class="p-button--positive is-dense"
            type="button"
            data-testid="save-artifact-btn"
            :disabled="saving"
            @click="save"
          >{{ saving ? 'Saving…' : 'Save' }}</button>
        </div>
      </div>

      <div v-if="error" class="p-notification--negative artifact-editor__error" data-testid="artifact-editor-error">
        <div class="p-notification__content">
          <p class="p-notification__message">{{ error }}</p>
        </div>
      </div>

      <div ref="containerRef" class="artifact-editor__container" data-testid="artifact-editor-container" />
    </div>
  </div>
</template>

<script setup>
import { ref, watch, onMounted, onBeforeUnmount, nextTick, shallowRef } from 'vue';
import { getArtifact, proposeArtifactChange } from '../../lib/api.js';

const props = defineProps({
  projectId:    { type: String, required: true },
  deploymentId: { type: String, required: true },
  artifactId:   { type: String, default: null },
});
const emit = defineEmits(['saved']);

const containerRef = ref(null);
const artifact = ref(null);
const loading  = ref(false);
const dirty    = ref(false);
const saving   = ref(false);
const error    = ref(null);

let editor     = null;
let monacoApi  = null;
let originalContent = '';

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

function languageForKind(kind) {
  if (kind === 'terraform' || kind === 'terragrunt') return 'json';
  if (kind === 'bash') return 'shell';
  return 'markdown';
}

async function loadArtifact() {
  if (!props.artifactId) {
    artifact.value = null;
    dirty.value = false;
    return;
  }
  loading.value = true;
  error.value = null;
  dirty.value = false;
  try {
    const a = await getArtifact(props.artifactId);
    artifact.value = a;
    originalContent = a.content ?? '';
    loading.value = false;
    await nextTick();
    mountEditor();
  } catch (e) {
    artifact.value = null;
    error.value = e.message || 'Failed to load artifact';
    loading.value = false;
  }
}

async function mountEditor() {
  if (editor) {
    editor.dispose();
    editor = null;
  }
  if (!containerRef.value || !artifact.value) return;
  if (!monacoApi) {
    monacoApi = await import('monaco-editor');
  }
  editor = monacoApi.editor.create(containerRef.value, {
    value: originalContent,
    language: languageForKind(artifact.value.kind),
    automaticLayout: true,
    minimap: { enabled: false },
    fontSize: 13,
    lineNumbers: 'on',
    wordWrap: 'on',
    scrollBeyondLastLine: false,
  });
  editor.onDidChangeModelContent(() => {
    const current = editor.getValue();
    dirty.value = current !== originalContent;
  });
}

async function save() {
  if (!editor || !dirty.value || saving.value) return;
  saving.value = true;
  error.value = null;
  const proposedContent = editor.getValue();
  try {
    await proposeArtifactChange(props.projectId, props.deploymentId, {
      artifact_id:      props.artifactId,
      source:           'user',
      explanation:       'Manual edit from the Deploy page',
      current_content:  originalContent,
      proposed_content: proposedContent,
    });
    originalContent = proposedContent;
    dirty.value = false;
    emit('saved');
  } catch (e) {
    error.value = e.message || 'Failed to save';
  } finally {
    saving.value = false;
  }
}

function handleKeydown(e) {
  if (e.ctrlKey && (e.key === 's' || e.key === 'S')) {
    e.preventDefault();
    save();
  }
}

watch(() => props.artifactId, () => loadArtifact());

onMounted(() => {
  loadArtifact();
  window.addEventListener('keydown', handleKeydown);
});

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleKeydown);
  if (editor) {
    editor.dispose();
    editor = null;
  }
});

defineExpose({ handleKeydown });
</script>

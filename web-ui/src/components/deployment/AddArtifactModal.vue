<template>
  <div v-if="open" class="modal" @click.self="$emit('close')">
    <div class="modal-content modal-content--wide" data-testid="add-artifact-modal">
      <button class="modal-close" type="button" @click="$emit('close')">✕</button>
      <h3>Add artifact</h3>
      <p class="modal-lede">Create a new artifact and add it to the deployment execution plan.</p>

      <div class="deploy-artifacts-modal-tabs" data-testid="add-artifact-tabs">
        <button
          class="deploy-artifacts-modal-tab"
          :class="{ 'deploy-artifacts-modal-tab--active': tab === 'content' }"
          data-testid="add-artifact-tab-content"
          @click="tab = 'content'"
        >Content</button>
        <button
          class="deploy-artifacts-modal-tab"
          :class="{ 'deploy-artifacts-modal-tab--active': tab === 'generate' }"
          data-testid="add-artifact-tab-generate"
          @click="tab = 'generate'"
        >Generate</button>
      </div>

      <div class="form-group">
        <label for="add-artifact-name">Name</label>
        <input
          id="add-artifact-name"
          v-model="name"
          type="text"
          data-testid="add-artifact-name"
          placeholder="Artifact name"
        />
      </div>

      <div class="form-group">
        <label for="add-artifact-kind">Kind</label>
        <select id="add-artifact-kind" v-model="kind" data-testid="add-artifact-kind">
          <option value="bash">Bash</option>
          <option value="terraform">Terraform</option>
          <option value="terragrunt">Terragrunt</option>
          <option value="markdown">Markdown</option>
        </select>
      </div>

      <div v-if="tab === 'content'" class="form-group">
        <label for="add-artifact-content">Content</label>
        <textarea
          id="add-artifact-content"
          v-model="content"
          rows="10"
          data-testid="add-artifact-content"
          placeholder="Artifact content"
        />
      </div>

      <div v-if="tab === 'generate'" class="form-group">
        <label for="add-artifact-prompt">Prompt</label>
        <textarea
          id="add-artifact-prompt"
          v-model="prompt"
          rows="10"
          data-testid="add-artifact-prompt"
          placeholder="Describe what this artifact should do..."
        />
      </div>

      <div v-if="generating" class="deploy-artifacts-modal-generating" data-testid="add-artifact-generating">
        <LoadingSpinner text="Generating artifact…" />
      </div>

      <div v-if="error" class="p-notification--negative">
        <div class="p-notification__content">
          <p class="p-notification__message">{{ error }}</p>
        </div>
      </div>

      <div class="modal-actions" v-if="!generating">
        <button class="p-button--base is-dense" type="button" @click="$emit('close')">Cancel</button>
        <button
          v-if="tab === 'content'"
          class="p-button--positive is-dense"
          type="button"
          data-testid="add-artifact-submit"
          :disabled="!name.trim() || submitting"
          @click="submitContent"
        >{{ submitting ? 'Adding…' : 'Add' }}</button>
        <button
          v-if="tab === 'generate'"
          class="p-button--positive is-dense"
          type="button"
          data-testid="add-artifact-generate-btn"
          :disabled="!name.trim() || !prompt.trim()"
          @click="submitGenerate"
        >Generate</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch } from 'vue';
import {
  createProjectArtifact, proposeProvisionChange,
} from '../../lib/api.js';
import LoadingSpinner from './LoadingSpinner.vue';

const props = defineProps({
  open:         { type: Boolean, default: false },
  projectId:    { type: String, required: true },
  deploymentId: { type: String, required: true },
});

const emit = defineEmits(['close', 'added']);

const tab       = ref('content');
const name      = ref('');
const kind      = ref('bash');
const content   = ref('');
const prompt    = ref('');
const submitting = ref(false);
const generating = ref(false);
const error     = ref(null);

watch(() => props.open, (v) => {
  if (v) {
    tab.value = 'content';
    name.value = '';
    kind.value = 'bash';
    content.value = '';
    prompt.value = '';
    submitting.value = false;
    generating.value = false;
    error.value = null;
  }
});

function inferFileExtension(kind) {
  if (kind === 'terraform') return '.tf';
  if (kind === 'terragrunt') return '.tf';
  if (kind === 'bash') return '.sh';
  if (kind === 'markdown') return '.md';
  return '';
}

async function submitContent() {
  if (!name.value.trim() || submitting.value) return;
  submitting.value = true;
  error.value = null;
  try {
    let artifactContent = content.value;
    if (kind.value === 'terraform' || kind.value === 'terragrunt') {
      const ext = inferFileExtension(kind.value);
      artifactContent = JSON.stringify({ [`${name.value}${ext}`]: content.value }, null, 2);
    }
    const result = await createProjectArtifact(props.projectId, {
      title: name.value.trim(),
      kind: kind.value,
      content: artifactContent,
    });
    emit('added', { id: result.id, kind: kind.value, content: artifactContent });
    emit('close');
  } catch (e) {
    error.value = e.message || 'Failed to create artifact';
  } finally {
    submitting.value = false;
  }
}

async function submitGenerate() {
  if (!name.value.trim() || !prompt.value.trim() || generating.value) return;
  generating.value = true;
  error.value = null;
  try {
    const result = await proposeProvisionChange(props.projectId, props.deploymentId, {
      instructions: `Add a new ${kind.value} artifact named "${name.value.trim()}": ${prompt.value.trim()}`,
    });
    const proposed = result.proposed_files ?? {};
    const ext = inferFileExtension(kind.value);
    const fileName = `${name.value.trim()}${ext}`;
    let artifactContent = proposed[fileName] ?? proposed[Object.keys(proposed)[0]] ?? '';
    if (kind.value === 'terraform' || kind.value === 'terragrunt') {
      const allFiles = { ...proposed };
      if (!allFiles[fileName]) allFiles[fileName] = artifactContent;
      artifactContent = JSON.stringify(allFiles, null, 2);
    }
    const created = await createProjectArtifact(props.projectId, {
      title: name.value.trim(),
      kind: kind.value,
      content: artifactContent,
    });
    emit('added', { id: created.id, kind: kind.value, content: artifactContent });
    emit('close');
  } catch (e) {
    error.value = e.message || 'Failed to generate artifact';
  } finally {
    generating.value = false;
  }
}
</script>

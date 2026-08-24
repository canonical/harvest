<template>
  <div class="design-panel">
    <div class="design-panel__actions">
      <button
        v-if="!deployment.design_doc"
        class="p-button--positive is-dense"
        data-testid="generate-design-btn"
        type="button"
        :disabled="busy"
        @click="generate"
      >{{ busy ? 'Generating…' : 'Generate design' }}</button>
      <template v-else>
        <button
          class="p-button--base is-dense"
          data-testid="edit-design-btn"
          type="button"
          :disabled="busy"
          @click="startEdit"
        >{{ editing ? 'Close editor' : 'Edit' }}</button>
      </template>
      <BusyStatus v-if="busyLabel" :text="busyLabel" />
    </div>

    <div v-if="error" class="p-notification--negative">
      <div class="p-notification__content">
        <p class="p-notification__message">{{ error }}</p>
      </div>
    </div>

    <template v-if="deployment.design_doc">
      <div class="design-panel__body">
        <div class="design-panel__main">
          <textarea
            v-if="editing"
            v-model="editContent"
            class="design-panel__editor"
            data-testid="design-editor"
          />
          <div
            v-else
            class="design-panel__preview"
            data-testid="design-preview"
            v-html="renderedDesign"
          />
        </div>

        <div class="design-panel__sidebar">
          <div v-if="editing" class="design-panel__edit-actions">
            <button class="p-button--base is-dense" type="button" data-testid="cancel-edit-btn" @click="cancelEdit">Cancel</button>
            <button class="p-button--positive is-dense" type="button" data-testid="save-design-btn" :disabled="saving" @click="saveEdit">
              {{ saving ? 'Saving…' : 'Save' }}
            </button>
          </div>
          <div class="design-panel__prompt">
            <label for="design-prompt">Propose a change</label>
            <textarea
              id="design-prompt"
              v-model="promptText"
              rows="4"
              data-testid="design-prompt"
              placeholder="Describe what you'd like to change"
            />
            <button
              class="p-button--base is-dense"
              type="button"
              data-testid="propose-design-btn"
              :disabled="!promptText.trim() || proposing"
              @click="propose"
            >{{ proposing ? 'Proposing…' : 'Propose' }}</button>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { renderMarkdown } from '../../lib/markdown.js';
import { getArtifact, generateDesign, updateArtifact, proposeArtifactChange } from '../../lib/api.js';
import BusyStatus from './BusyStatus.vue';

const props = defineProps({
  projectId:  { type: String, required: true },
  deployment: { type: Object, required: true },
});
const emit = defineEmits(['refresh']);

const designContent = ref('');
const editing         = ref(false);
const editContent    = ref('');
const saving          = ref(false);
const busy            = ref(false);
const proposing      = ref(false);
const promptText    = ref('');
const error          = ref(null);

const renderedDesign = computed(() => designContent.value ? renderMarkdown(designContent.value, {}, {}) : '');

const busyLabel = computed(() => {
  if (busy.value)     return 'Generating design…';
  if (saving.value)   return 'Saving design…';
  if (proposing.value) return 'Proposing a change…';
  return null;
});

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

async function generate() {
  busy.value = true;
  error.value = null;
  try {
    await generateDesign(props.projectId, props.deployment.id);
    emit('refresh');
  } catch (e) {
    error.value = e.message || 'Failed to generate design';
  } finally {
    busy.value = false;
  }
}

function startEdit() {
  if (editing.value) {
    editing.value = false;
    return;
  }
  editContent.value = designContent.value;
  editing.value = true;
}

function cancelEdit() {
  editing.value = false;
  editContent.value = '';
}

async function saveEdit() {
  saving.value = true;
  error.value = null;
  try {
    await updateArtifact(props.deployment.design_doc.id, {
      title: props.deployment.design_doc.title ?? 'Design',
      kind: 'markdown',
      content: editContent.value,
    });
    designContent.value = editContent.value;
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
  } catch (e) {
    error.value = e.message || 'Failed to propose change';
  } finally {
    proposing.value = false;
  }
}

watch(() => props.deployment.design_doc?.id, loadDesignContent, { immediate: true });
</script>

<template>
  <div class="guide-panel">
    <div class="guide-panel__actions">
      <button
        class="p-button--base is-dense"
        data-testid="edit-guide-btn"
        type="button"
        @click="startEdit"
      >{{ editing ? 'Close editor' : 'Edit' }}</button>
    </div>

    <div v-if="!deployment.guide" class="guide-panel__empty">
      <p>No guide yet.</p>
    </div>

    <template v-else>
      <div class="guide-panel__body">
        <textarea
          v-if="editing"
          v-model="editContent"
          class="guide-panel__editor"
          data-testid="guide-editor"
        />
        <div
          v-else
          class="guide-panel__preview"
          data-testid="guide-preview"
          v-html="renderedGuide"
        />
      </div>

      <div v-if="editing" class="guide-panel__edit-actions">
        <button class="p-button--base is-dense" type="button" @click="cancelEdit">Cancel</button>
        <button
          class="p-button--positive is-dense"
          type="button"
          data-testid="save-guide-btn"
          :disabled="saving"
          @click="saveEdit"
        >{{ saving ? 'Saving…' : 'Save' }}</button>
      </div>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { renderMarkdown } from '../../lib/markdown.js';
import { getArtifact, updateArtifact } from '../../lib/api.js';

const props = defineProps({
  projectId:  { type: String, required: true },
  deployment: { type: Object, required: true },
});
const emit = defineEmits(['refresh']);

const guideContent = ref('');
const editing        = ref(false);
const editContent    = ref('');
const saving         = ref(false);

const renderedGuide = computed(() => guideContent.value ? renderMarkdown(guideContent.value, {}, {}) : '');

async function loadGuideContent() {
  if (!props.deployment.guide) {
    guideContent.value = '';
    return;
  }
  try {
    const artifact = await getArtifact(props.deployment.guide.id);
    guideContent.value = artifact.content || '';
  } catch {
    guideContent.value = '';
  }
}

function startEdit() {
  if (editing.value) {
    editing.value = false;
    return;
  }
  editContent.value = guideContent.value;
  editing.value = true;
}

function cancelEdit() {
  editing.value = false;
  editContent.value = '';
}

async function saveEdit() {
  saving.value = true;
  try {
    await updateArtifact(props.deployment.guide.id, {
      title: props.deployment.guide.title ?? 'Guide',
      kind: 'markdown',
      content: editContent.value,
    });
    guideContent.value = editContent.value;
    editing.value = false;
    emit('refresh');
  } finally {
    saving.value = false;
  }
}

watch(() => props.deployment.guide?.id, loadGuideContent, { immediate: true });
</script>

<template>
  <div class="context-panel">
    <div class="context-panel__list">
      <div class="context-panel__list-header">
        <span>Context artifacts</span>
        <span class="context-panel__count">{{ artifacts.length }}</span>
      </div>
      <div class="context-panel__list-scroll">
        <p v-if="!artifacts.length" class="context-panel__empty">No context artifacts yet.</p>
        <button
          v-for="a in artifacts"
          :key="a.id"
          type="button"
          class="context-panel__item"
          :class="{ 'context-panel__item--active': selectedId === a.id }"
          :data-testid="`context-artifact-${a.id}`"
          @click="select(a.id)"
        >
          <span class="context-panel__item-title">{{ a.title }}</span>
          <span class="context-panel__item-kind">{{ a.kind }}</span>
          <button
            class="context-panel__item-remove"
            type="button"
            :data-testid="`remove-context-${a.id}`"
            @click.stop="remove(a.id)"
          >✕</button>
        </button>
      </div>

      <div class="context-panel__add">
        <input v-model="newTitle" type="text" placeholder="Title" data-testid="add-context-title" class="context-panel__add-title" />
        <select v-model="newKind" data-testid="add-context-kind" class="context-panel__add-kind">
          <option value="markdown">Markdown</option>
          <option value="bash">Bash</option>
          <option value="terraform">Terraform</option>
          <option value="terragrunt">Terragrunt</option>
          <option value="pdf">PDF</option>
        </select>
        <textarea
          v-model="newContent"
          rows="3"
          data-testid="add-context-content"
          class="context-panel__add-content"
          :placeholder="contentPlaceholder"
        />
        <div v-if="addError" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ addError }}</p>
          </div>
        </div>
        <button
          class="p-button--positive is-dense"
          type="button"
          data-testid="add-context-submit"
          :disabled="!canAdd || adding"
          @click="add"
        >{{ adding ? 'Adding…' : 'Add' }}</button>
      </div>
    </div>

    <div class="context-panel__viewer">
      <BusyStatus v-if="selectedId && loadingContent" text="Loading…" />
      <template v-else-if="selectedId && selectedContent !== null">
        <div class="context-panel__viewer-header">
          <span>{{ selectedTitle }}</span>
          <span class="context-panel__viewer-kind">{{ selectedKind }}</span>
        </div>
        <pre class="context-panel__viewer-content" :data-testid="`content-context-${selectedId}`">{{ selectedContent }}</pre>
      </template>
      <p v-else class="context-panel__viewer-empty">Select an artifact to view its content.</p>
    </div>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue';
import { addContextArtifact, removeContextArtifact, getArtifact } from '../../lib/api.js';
import BusyStatus from './BusyStatus.vue';

const props = defineProps({
  projectId:  { type: String, required: true },
  deployment: { type: Object, required: true },
});
const emit = defineEmits(['refresh']);

const artifacts = computed(() => props.deployment.context_artifacts ?? []);

const selectedId       = ref(null);
const selectedContent   = ref(null);
const selectedTitle     = ref('');
const selectedKind      = ref('');
const loadingContent    = ref(false);

const newTitle    = ref('');
const newKind     = ref('markdown');
const newContent  = ref('');
const adding      = ref(false);
const addError    = ref(null);

const canAdd = computed(() => newTitle.value.trim().length > 0);

const contentPlaceholder = computed(() => {
  switch (newKind.value) {
    case 'bash':        return '#!/usr/bin/env bash\necho hello';
    case 'terraform':
    case 'terragrunt':  return '{"main.tf": "resource ..."}';
    default:            return 'Write markdown here…';
  }
});

async function select(id) {
  if (selectedId.value === id) {
    selectedId.value = null;
    selectedContent.value = null;
    return;
  }
  selectedId.value = id;
  selectedContent.value = null;
  loadingContent.value = true;
  const artifact = artifacts.value.find(a => a.id === id);
  selectedTitle.value = artifact?.title ?? '';
  selectedKind.value = artifact?.kind ?? '';
  try {
    const full = await getArtifact(id);
    selectedContent.value = full.content || '';
  } catch {
    selectedContent.value = '';
  } finally {
    loadingContent.value = false;
  }
}

async function add() {
  if (!canAdd.value) return;
  adding.value = true;
  addError.value = null;
  try {
    await addContextArtifact(props.projectId, props.deployment.id, {
      title: newTitle.value.trim(),
      kind: newKind.value,
      content: newContent.value,
    });
    newTitle.value = '';
    newContent.value = '';
    emit('refresh');
  } catch (e) {
    addError.value = e.message || 'Failed to add';
  } finally {
    adding.value = false;
  }
}

async function remove(id) {
  try {
    await removeContextArtifact(props.projectId, props.deployment.id, id);
    if (selectedId.value === id) {
      selectedId.value = null;
      selectedContent.value = null;
    }
    emit('refresh');
  } catch (e) {
    addError.value = e.message || 'Failed to remove';
  }
}
</script>

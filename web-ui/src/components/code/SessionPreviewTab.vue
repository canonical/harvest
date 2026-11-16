<template>
  <div class="session-tab">
    <div class="session-tab__bar">
      <select v-if="workspace.previews.length" class="session-tab__picker" :value="preview?.id" aria-label="Preview" @change="switchTo($event.target.value)">
        <option v-for="entry in workspace.previews" :key="entry.id" :value="entry.id">{{ describe(entry) }}</option>
      </select>
      <button class="p-button--base is-dense" type="button" @click="creating = !creating">New preview</button>
      <template v-if="preview && !creating">
        <label class="session-tab__inline-label" :for="`share-${tab.tabId}`">Shared with</label>
        <select :id="`share-${tab.tabId}`" :value="preview.audience" @change="share($event.target.value)">
          <option v-for="audience in AUDIENCES" :key="audience.id" :value="audience.id">{{ audience.label }}</option>
        </select>
        <button v-if="preview.audience !== 'owner'" class="p-button--base is-dense" type="button" @click="copyLink">{{ copied ? 'Copied' : 'Copy link' }}</button>
        <button class="p-button--base is-dense" type="button" @click="remove">Delete preview</button>
      </template>
    </div>
    <p v-if="error" class="code-error" role="alert">{{ error }}</p>
    <div class="session-tab__body session-tab__body--scroll">
      <PreviewForm v-if="creating || !workspace.previews.length" :create="workspace.createPreview" :cancellable="creating" @created="onCreated" @cancel="creating = false" />
      <PreviewPanel v-else-if="preview" :key="preview.id" :preview-id="preview.id" />
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import PreviewPanel from './PreviewPanel.vue';
import PreviewForm from './PreviewForm.vue';
import { useLayoutWorkspace } from '../../lib/layout-workspace.js';
import { AUDIENCES, shareUrl } from '../../lib/preview-api.js';
import { expiresIn, previewLabel } from '../../lib/session-tabs.js';

const props = defineProps({ tab: { type: Object, required: true } });

const REFRESH_MS = 15000;
const workspace = useLayoutWorkspace();
const creating = ref(false);
const copied = ref(false);
const error = ref('');
let timer = null;

const preview = computed(() => workspace.previews.find(p => p.id === props.tab.previewId) ?? null);

function describe(entry) {
  const from = entry.chat_title ? ` · from “${entry.chat_title}”` : '';
  return `${previewLabel(entry)}${from} · ${expiresIn(entry)}`;
}

function switchTo(previewId) {
  if (previewId !== props.tab.previewId) workspace.updateTab(props.tab.tabId, { previewId });
}

watch(
  () => [props.tab.previewId, workspace.previews.length],
  () => {
    if (!preview.value && workspace.previews.length) switchTo(workspace.previews[0].id);
  },
  { immediate: true },
);

async function attempt(action) {
  error.value = '';
  try {
    await action();
  } catch (e) {
    error.value = e.message;
  }
}

function onCreated(created) {
  creating.value = false;
  switchTo(created.id);
}

function share(audience) {
  return attempt(() => workspace.shareWith(preview.value.id, audience));
}

function remove() {
  if (!window.confirm('Delete this preview? Its app stops, and shared links stop working.')) return;
  return attempt(() => workspace.deletePreview(preview.value.id));
}

async function copyLink() {
  try {
    await navigator.clipboard.writeText(shareUrl(preview.value.id));
    copied.value = true;
    setTimeout(() => { copied.value = false; }, 1500);
  } catch {
    copied.value = false;
  }
}

function refresh() {
  workspace.refreshPreviews().catch(() => {});
}

onMounted(() => {
  refresh();
  timer = setInterval(refresh, REFRESH_MS);
});
onUnmounted(() => clearInterval(timer));
</script>

<template>
  <div class="session-tab">
    <div v-if="!workspace.previews.length" class="session-tab__empty preview-empty">
      <h4>No preview yet</h4>
      <p>A preview lets you, and anyone you share it with, try the work: a web app shown here, an HTTP API to send requests to, or a command-line tool.</p>
      <div class="preview-empty__actions">
        <button class="p-button--positive is-dense" type="button" :disabled="asking" @click="askForPreview">Ask the agent</button>
        <button class="p-button--base is-dense" type="button" @click="creating = true">Start one yourself…</button>
      </div>
    </div>
    <PreviewPanel
      v-else-if="preview"
      ref="panel"
      :key="preview.id"
      :preview-id="preview.id"
      :extra-menu-items="MENU_ITEMS"
      @menu="onMenu"
    >
      <template #actions>
        <button v-if="expiringSoon" class="p-chip preview-expiry" type="button" title="Keep this preview for another day" @click="keep">
          <span class="p-chip__value">{{ expiresIn(preview) }} · Keep</span>
        </button>
        <button class="p-button--base is-dense" type="button" @click="sharing = true">
          Share<span v-if="preview.audience !== 'owner'" class="p-status-label--information preview-shared">Shared</span>
        </button>
      </template>
      <template #failure-actions="{ failure }">
        <button class="p-button--positive is-dense u-no-margin--bottom" type="button" @click="askToFix(failure)">Ask the agent to fix this</button>
      </template>
    </PreviewPanel>

    <NewPreviewModal v-if="creating" :create="workspace.createPreview" :previews="workspace.previews" @close="creating = false" @created="onCreated" />
    <SharePreviewModal v-if="sharing && preview" :preview="preview" :share="workspace.shareWith" @close="sharing = false" />
    <PreviewLogModal v-if="showingLog && preview" :preview-id="preview.id" @close="showingLog = false" />
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import PreviewPanel from './PreviewPanel.vue';
import NewPreviewModal from './NewPreviewModal.vue';
import SharePreviewModal from './SharePreviewModal.vue';
import PreviewLogModal from './PreviewLogModal.vue';
import { useLayoutWorkspace } from '../../lib/layout-workspace.js';
import { expiresIn, previewLabel } from '../../lib/session-tabs.js';
import { keepPreview, restartPreview } from '../../lib/code-api.js';
import { confirm } from '../../composables/useConfirm.js';
import { showToast } from '../../composables/useToasts.js';

const props = defineProps({
  tab: { type: Object, required: true },
  paneId: { type: String, required: true },
});

const REFRESH_MS = 15000;
const HOUR_MS = 3_600_000;
const MENU_ITEMS = [
  { id: 'restart', label: 'Restart the app' },
  { id: 'log', label: 'View start log' },
  { id: 'menu-divider', divider: true },
  { id: 'delete', label: 'Delete preview…', danger: true },
];

const workspace = useLayoutWorkspace();
const panel = ref(null);
const creating = ref(false);
const sharing = ref(false);
const showingLog = ref(false);
const asking = ref(false);
const now = ref(Date.now());
let timer = null;

const preview = computed(() => workspace.previews.find(p => p.id === props.tab.previewId) ?? null);
const expiringSoon = computed(() => preview.value && new Date(preview.value.expires_at).getTime() - now.value < HOUR_MS);

function show(previewId) {
  if (previewId !== props.tab.previewId) workspace.updateTab(props.tab.tabId, { previewId });
}

watch(
  () => [props.tab.previewId, workspace.previews.length],
  () => {
    if (!preview.value && workspace.previews.length) show(workspace.previews[0].id);
  },
  { immediate: true },
);

watch(() => workspace.tabRequest, (request) => {
  if (request?.tabId === props.tab.tabId && request.action === 'new-preview') creating.value = true;
});

async function attempt(action, success) {
  try {
    await action();
    if (success) showToast({ message: success });
  } catch (e) {
    showToast({ message: e.message, kind: 'negative' });
  }
}

function onCreated(created) {
  creating.value = false;
  show(created.id);
  showToast({ message: `${previewLabel(created)} is running` });
}

function askForPreview() {
  asking.value = true;
  attempt(() => workspace.askAgent(props.paneId, 'Set up a preview so I can try this work.')).finally(() => {
    asking.value = false;
  });
}

function askToFix(failure) {
  const label = previewLabel(preview.value);
  attempt(() => workspace.askAgent(props.paneId, `${label} does not start:\n\n${failure}\n\nPlease fix it, then configure the preview again.`));
}

async function onMenu(item) {
  if (item.id === 'log') {
    showingLog.value = true;
  } else if (item.id === 'restart') {
    await attempt(() => restartPreview(preview.value.id), 'The app restarted');
    panel.value?.retry();
  } else if (item.id === 'delete') {
    const confirmed = await confirm({
      title: 'Delete preview?',
      message: 'Its app stops, copies made for other people are removed, and shared links stop working.',
      confirmLabel: 'Delete preview',
      danger: true,
    });
    if (confirmed) await attempt(() => workspace.deletePreview(preview.value.id), 'Preview deleted');
  }
}

function keep() {
  attempt(async () => {
    await keepPreview(preview.value.id);
    await workspace.refreshPreviews();
  }, 'Kept for another day');
}

function refresh() {
  now.value = Date.now();
  workspace.refreshPreviews().catch(() => {});
}

onMounted(() => {
  refresh();
  timer = setInterval(refresh, REFRESH_MS);
});
onUnmounted(() => clearInterval(timer));
</script>

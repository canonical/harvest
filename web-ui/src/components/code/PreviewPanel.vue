<template>
  <div class="preview-panel">
    <div class="preview-toolbar">
      <form v-if="framed" class="preview-toolbar__address" @submit.prevent="showInFrame">
        <input v-model="path" class="preview-toolbar__path" type="text" aria-label="Path in the app" spellcheck="false" />
      </form>
      <span v-else class="preview-toolbar__spacer" />
      <template v-if="isWeb && info?.web_enabled">
        <button v-if="framed" class="p-button--base is-dense has-icon" type="button" aria-label="Reload" title="Reload" @click="showInFrame">
          <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><polyline points="21 3 21 8 16 8"/></svg>
        </button>
        <button class="p-button--base is-dense has-icon" type="button" aria-label="Open in new tab" title="Open in new tab" :disabled="opening" @click="openApp">
          <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
        </button>
        <MenuButton v-if="framed" label="Width" :title="`Width: ${widthLabel}`" :items="widthItems" button-class="p-button--base is-dense" @select="(item) => (width = item.id)">
          {{ widthLabel }}
        </MenuButton>
      </template>
      <slot name="actions" />
      <MenuButton v-if="menuItems.length" label="More actions" :items="menuItems" button-class="p-button--base is-dense has-icon" @select="onMenu">
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="19" cy="12" r="2"/></svg>
      </MenuButton>
    </div>

    <div class="preview-panel__stage">
      <div v-if="failure" class="p-notification--negative preview-panel__failure">
        <div class="p-notification__content">
          <h5 class="p-notification__title">The app could not start</h5>
          <p class="p-notification__message">{{ failureSummary }}</p>
        </div>
        <div class="p-notification__actions">
          <button v-if="failureDetails" class="p-button--base is-dense u-no-margin--bottom" type="button" @click="showDetails = !showDetails">{{ showDetails ? 'Hide output' : 'Show output' }}</button>
          <slot name="failure-actions" :failure="failure" />
          <button class="p-button--base is-dense u-no-margin--bottom" type="button" @click="retry">Try again</button>
        </div>
        <pre v-if="showDetails" class="code-log">{{ failureDetails }}</pre>
      </div>
      <div v-else-if="!info || info.status === 'starting'" class="preview-panel__waiting">
        <SegmentMark active />
        <span>{{ !info ? 'Loading…' : info.is_owner ? 'Waiting for the session…' : 'Starting your own copy of the app…' }}</span>
      </div>
      <p v-else-if="info.status === 'ended'" class="u-text--muted">This preview ended with its session.</p>
      <template v-else-if="isWeb">
        <div v-if="framed" class="preview-frame-stage">
          <iframe
            v-if="frameUrl"
            class="preview-frame"
            :class="`preview-frame--${width}`"
            :src="frameUrl"
            title="Preview of the app"
            sandbox="allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox allow-downloads allow-modals allow-same-origin"
          />
        </div>
        <div v-else-if="info.web_enabled && info.framable" class="p-notification--caution">
          <div class="p-notification__content">
            <h5 class="p-notification__title">The app cannot be shown inside this page</h5>
            <p class="p-notification__message">Previews appear inside Harvest only when it is opened at <a :href="info.harvest_origin">{{ info.harvest_origin }}</a>. The app still opens in its own tab.</p>
          </div>
          <div class="p-notification__actions">
            <button class="p-button is-dense u-no-margin--bottom" type="button" :disabled="opening" @click="openApp">{{ opening ? 'Opening…' : 'Open the app' }}</button>
          </div>
        </div>
        <div v-else-if="info.web_enabled" class="preview-panel__web">
          <p>The app runs at its own address, exactly as it does locally.</p>
          <button class="p-button--positive" type="button" :disabled="opening" @click="openApp">{{ opening ? 'Opening…' : 'Open the app' }}</button>
        </div>
        <p v-else class="u-text--muted">Web previews are turned off on this server. An administrator enables them by setting <code>preview_domain</code> under <code>[code]</code>.</p>
      </template>
      <ApiRunner v-else-if="info.kind === 'api'" :preview-id="previewId" />
      <CliRunner v-else :preview-id="previewId" />
    </div>
  </div>
</template>

<script setup>
import { computed, ref, watch, onUnmounted } from 'vue';
import SegmentMark from '../chat/SegmentMark.vue';
import MenuButton from '../ui/MenuButton.vue';
import ApiRunner from './ApiRunner.vue';
import CliRunner from './CliRunner.vue';
import { canFrameHere, enterPreview, getPreview, resetPreview } from '../../lib/preview-api.js';
import { splitFailure } from '../../lib/preview-failure.js';
import { showToast } from '../../composables/useToasts.js';

const props = defineProps({
  previewId: { type: String, required: true },
  extraMenuItems: { type: Array, default: () => [] },
});
const emit = defineEmits(['loaded', 'menu']);

const POLL_MS = 2500;
const WIDTHS = [
  { id: 'desktop', label: 'Desktop' },
  { id: 'tablet', label: 'Tablet' },
  { id: 'phone', label: 'Phone' },
];

const info = ref(null);
const failure = ref('');
const showDetails = ref(false);
const opening = ref(false);
const path = ref('/');
const width = ref('desktop');
const frameUrl = ref('');
let timer = null;

const isWeb = computed(() => info.value?.kind === 'web');
const framed = computed(() => isWeb.value && info.value?.web_enabled && info.value?.framable && canFrameHere(info.value));
const widthLabel = computed(() => WIDTHS.find(w => w.id === width.value).label);
const widthItems = computed(() => WIDTHS.map(w => ({ ...w, checked: w.id === width.value })));
const failureSummary = computed(() => splitFailure(failure.value).summary);
const failureDetails = computed(() => splitFailure(failure.value).details);
const menuItems = computed(() => [
  ...(info.value && !info.value.is_owner ? [{ id: 'fresh-copy', label: 'Fresh copy of the app' }] : []),
  ...props.extraMenuItems,
]);

function fail(message) {
  failure.value = message;
  showDetails.value = false;
}

async function load() {
  clearTimeout(timer);
  try {
    info.value = await getPreview(props.previewId);
    emit('loaded', info.value);
    if (info.value.status === 'failed') fail(info.value.error ?? 'The copy of the app failed.');
    if (info.value.status === 'starting') timer = setTimeout(load, POLL_MS);
    if (info.value.status === 'ready' && framed.value && !frameUrl.value) await showInFrame();
  } catch (e) {
    fail(e.message);
  }
}

async function showInFrame() {
  try {
    frameUrl.value = '';
    frameUrl.value = (await enterPreview(props.previewId, path.value)).url;
  } catch (e) {
    fail(e.message);
  }
}

async function openApp() {
  opening.value = true;
  const tab = window.open('', '_blank');
  try {
    const { url } = await enterPreview(props.previewId, path.value);
    if (tab) {
      tab.opener = null;
      tab.location.href = url;
    } else {
      window.location.href = url;
    }
  } catch (e) {
    tab?.close();
    fail(e.message);
  } finally {
    opening.value = false;
  }
}

async function retry() {
  failure.value = '';
  frameUrl.value = '';
  await load();
}

async function onMenu(item) {
  if (item.id !== 'fresh-copy') {
    emit('menu', item);
    return;
  }
  try {
    await resetPreview(props.previewId);
    showToast({ message: 'Starting a fresh copy of the app' });
    await retry();
  } catch (e) {
    showToast({ message: e.message, kind: 'negative' });
  }
}

defineExpose({ retry });

watch(() => props.previewId, retry, { immediate: true });
onUnmounted(() => clearTimeout(timer));
</script>

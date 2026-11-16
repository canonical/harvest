<template>
  <div class="preview-panel">
    <p v-if="error" class="code-error" role="alert">{{ error }}</p>
    <template v-else-if="info">
      <div v-if="info.status === 'starting'" class="preview-panel__waiting">
        <SegmentMark active />
        <span>{{ info.is_owner ? 'Waiting for the session…' : 'Starting your own copy of the app…' }}</span>
      </div>
      <pre v-else-if="info.status === 'failed'" class="code-log">{{ info.error }}</pre>
      <p v-else-if="info.status === 'ended'" class="u-text--muted">This preview ended with its session.</p>
      <template v-else>
        <div class="preview-panel__toolbar">
          <button v-if="!info.is_owner" class="p-button--base is-dense" type="button" :disabled="resetting" @click="freshCopy">Fresh copy</button>
        </div>
        <template v-if="info.kind === 'web' && info.web_enabled && info.framable">
          <form class="preview-frame-bar" @submit.prevent="showInFrame">
            <input v-model="path" class="preview-frame-bar__path" type="text" aria-label="Path in the app" spellcheck="false" />
            <button class="p-button--base is-dense" type="submit">Go</button>
            <button class="p-button--base is-dense" type="button" @click="showInFrame">Reload</button>
            <button class="p-button--base is-dense" type="button" :disabled="opening" @click="openApp">Open in new tab</button>
            <div class="preview-frame-bar__widths" role="group" aria-label="Width">
              <button
                v-for="option in WIDTHS"
                :key="option.id"
                class="p-button--base is-dense"
                type="button"
                :aria-pressed="String(width === option.id)"
                @click="width = option.id"
              >{{ option.label }}</button>
            </div>
          </form>
          <div class="preview-frame-stage">
            <iframe
              v-if="frameUrl"
              class="preview-frame"
              :class="`preview-frame--${width}`"
              :src="frameUrl"
              title="Preview of the app"
              sandbox="allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox allow-downloads allow-modals allow-same-origin"
            />
          </div>
        </template>
        <div v-else-if="info.kind === 'web'" class="preview-panel__web">
          <template v-if="info.web_enabled">
            <p>The app runs at its own address, exactly as it does locally.</p>
            <button class="p-button--positive" type="button" :disabled="opening" @click="openApp">{{ opening ? 'Opening…' : 'Open the app' }}</button>
          </template>
          <p v-else class="u-text--muted">Web previews are turned off on this server. An administrator enables them by setting <code>preview_domain</code> under <code>[code]</code>.</p>
        </div>
        <ApiRunner v-else-if="info.kind === 'api'" :preview-id="previewId" />
        <CliRunner v-else :preview-id="previewId" />
      </template>
    </template>
  </div>
</template>

<script setup>
import { ref, watch, onUnmounted } from 'vue';
import SegmentMark from '../chat/SegmentMark.vue';
import ApiRunner from './ApiRunner.vue';
import CliRunner from './CliRunner.vue';
import { enterPreview, getPreview, resetPreview } from '../../lib/preview-api.js';

const props = defineProps({ previewId: { type: String, required: true } });
const emit = defineEmits(['loaded']);

const POLL_MS = 2500;
const info = ref(null);
const error = ref('');
const resetting = ref(false);
const opening = ref(false);
const WIDTHS = [
  { id: 'desktop', label: 'Desktop' },
  { id: 'tablet', label: 'Tablet' },
  { id: 'phone', label: 'Phone' },
];
const path = ref('/');
const width = ref('desktop');
const frameUrl = ref('');
let timer = null;

async function load() {
  clearTimeout(timer);
  try {
    info.value = await getPreview(props.previewId);
    error.value = '';
    emit('loaded', info.value);
    if (info.value.status === 'starting') timer = setTimeout(load, POLL_MS);
    if (info.value.status === 'ready' && info.value.framable && !frameUrl.value) await showInFrame();
  } catch (e) {
    error.value = e.message;
  }
}

async function showInFrame() {
  try {
    frameUrl.value = '';
    frameUrl.value = (await enterPreview(props.previewId, path.value)).url;
  } catch (e) {
    error.value = e.message;
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
    error.value = e.message;
  } finally {
    opening.value = false;
  }
}

async function freshCopy() {
  resetting.value = true;
  try {
    await resetPreview(props.previewId);
    frameUrl.value = '';
    await load();
  } catch (e) {
    error.value = e.message;
  } finally {
    resetting.value = false;
  }
}

watch(() => props.previewId, load, { immediate: true });
onUnmounted(() => clearTimeout(timer));
</script>

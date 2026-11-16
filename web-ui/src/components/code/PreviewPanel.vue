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
        <div v-if="info.kind === 'web'" class="preview-panel__web">
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
let timer = null;

async function load() {
  clearTimeout(timer);
  try {
    info.value = await getPreview(props.previewId);
    error.value = '';
    emit('loaded', info.value);
    if (info.value.status === 'starting') timer = setTimeout(load, POLL_MS);
  } catch (e) {
    error.value = e.message;
  }
}

async function openApp() {
  opening.value = true;
  const tab = window.open('', '_blank');
  try {
    const { url } = await enterPreview(props.previewId);
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

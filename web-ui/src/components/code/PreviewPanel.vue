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
          <a v-if="info.kind === 'web'" :href="info.base_path" target="_blank" rel="noopener">Open in a new tab</a>
          <button v-if="!info.is_owner" class="p-button--base is-dense" type="button" :disabled="resetting" @click="freshCopy">Fresh copy</button>
        </div>
        <iframe v-if="info.kind === 'web'" :key="frameKey" class="preview-frame" :src="info.base_path" title="Preview" />
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
import { getPreview, resetPreview } from '../../lib/preview-api.js';

const props = defineProps({ previewId: { type: String, required: true } });
const emit = defineEmits(['loaded']);

const POLL_MS = 2500;
const info = ref(null);
const error = ref('');
const resetting = ref(false);
const frameKey = ref(0);
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

async function freshCopy() {
  resetting.value = true;
  try {
    await resetPreview(props.previewId);
    frameKey.value += 1;
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

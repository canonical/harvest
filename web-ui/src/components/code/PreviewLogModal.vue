<template>
  <AppModal title="Start log" size="wide" @close="emit('close')">
    <p v-if="loading" class="u-text--muted">Loading…</p>
    <p v-else-if="error" class="code-error" role="alert">{{ error }}</p>
    <pre v-else class="code-log">{{ log || 'The app has not printed anything.' }}</pre>
    <template #actions>
      <button class="p-button--base is-dense" type="button" @click="load">Refresh</button>
      <button class="p-button--positive is-dense" type="button" @click="emit('close')">Close</button>
    </template>
  </AppModal>
</template>

<script setup>
import { onMounted, ref } from 'vue';
import AppModal from '../ui/AppModal.vue';
import { previewLog } from '../../lib/code-api.js';

const props = defineProps({ previewId: { type: String, required: true } });
const emit = defineEmits(['close']);

const log = ref('');
const error = ref('');
const loading = ref(true);

async function load() {
  loading.value = true;
  error.value = '';
  try {
    log.value = (await previewLog(props.previewId)).log;
  } catch (e) {
    error.value = e.message;
  } finally {
    loading.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="preview-runner">
    <div ref="logEl" class="preview-terminal" aria-live="polite">
      <div v-for="(entry, i) in history" :key="i" class="preview-terminal__entry">
        <div class="preview-terminal__command">$ {{ entry.command }}</div>
        <pre v-if="entry.output">{{ entry.output }}</pre>
        <div v-if="entry.exitCode" class="preview-terminal__exit">exit code {{ entry.exitCode }}</div>
      </div>
      <div v-if="!history.length" class="u-text--muted">Commands run from the repository root of your own copy.</div>
    </div>
    <form class="preview-runner__line" @submit.prevent="run">
      <input v-model="command" type="text" aria-label="Command" placeholder="./my-tool --help" :disabled="busy" />
      <button class="p-button--positive is-dense" type="submit" :disabled="!command.trim() || busy">{{ busy ? 'Running…' : 'Run' }}</button>
    </form>
  </div>
</template>

<script setup>
import { ref, nextTick } from 'vue';
import { execPreview } from '../../lib/preview-api.js';

const props = defineProps({ previewId: { type: String, required: true } });

const command = ref('');
const history = ref([]);
const busy = ref(false);
const logEl = ref(null);

async function run() {
  const text = command.value.trim();
  busy.value = true;
  command.value = '';
  try {
    const result = await execPreview(props.previewId, text);
    history.value.push({ command: text, output: `${result.stdout}${result.stderr}`, exitCode: result.exit_code });
  } catch (e) {
    history.value.push({ command: text, output: e.message, exitCode: null });
  } finally {
    busy.value = false;
    await nextTick();
    if (logEl.value) logEl.value.scrollTop = logEl.value.scrollHeight;
  }
}
</script>

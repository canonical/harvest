<template>
  <div class="preview-runner">
    <form class="preview-runner__request" @submit.prevent="send">
      <div class="preview-runner__line">
        <select v-model="method" aria-label="Method">
          <option v-for="m in METHODS" :key="m" :value="m">{{ m }}</option>
        </select>
        <input v-model="path" type="text" aria-label="Path" placeholder="/api/items?limit=10" />
        <button class="p-button--positive is-dense" type="submit" :disabled="busy">{{ busy ? 'Sending…' : 'Send' }}</button>
      </div>
      <label :for="`${uid}-headers`">Headers</label>
      <textarea :id="`${uid}-headers`" v-model="headerText" rows="2" spellcheck="false" placeholder="Content-Type: application/json" />
      <template v-if="hasBody">
        <label :for="`${uid}-body`">Body</label>
        <textarea :id="`${uid}-body`" v-model="body" rows="5" spellcheck="false" />
      </template>
    </form>
    <p v-if="error" class="code-error" role="alert">{{ error }}</p>
    <div v-if="response" class="preview-runner__response">
      <p><strong>{{ response.status }}</strong> <span class="u-text--muted">· {{ response.elapsed_ms }} ms</span></p>
      <details>
        <summary>Headers</summary>
        <pre class="code-log">{{ formattedHeaders }}</pre>
      </details>
      <pre class="code-log">{{ formattedBody }}</pre>
      <p v-if="response.truncated" class="u-text--muted">The response was cut at 256 KB.</p>
    </div>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue';
import { sendPreviewRequest, parseHeaderLines } from '../../lib/preview-api.js';

const props = defineProps({ previewId: { type: String, required: true } });

const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];
const uid = `api-${Math.random().toString(36).slice(2, 8)}`;
const method = ref('GET');
const path = ref('/');
const headerText = ref('');
const body = ref('');
const busy = ref(false);
const error = ref('');
const response = ref(null);

const hasBody = computed(() => !['GET', 'HEAD', 'OPTIONS'].includes(method.value));
const formattedHeaders = computed(() =>
  Object.entries(response.value?.headers ?? {}).map(([k, v]) => `${k}: ${v}`).join('\n'));
const formattedBody = computed(() => {
  const text = response.value?.body ?? '';
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
});

async function send() {
  busy.value = true;
  error.value = '';
  try {
    response.value = await sendPreviewRequest(props.previewId, {
      method: method.value,
      path: path.value,
      headers: parseHeaderLines(headerText.value),
      body: hasBody.value ? body.value : null,
    });
  } catch (e) {
    error.value = e.message;
  } finally {
    busy.value = false;
  }
}
</script>

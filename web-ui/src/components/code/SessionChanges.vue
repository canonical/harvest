<template>
  <div class="session-tab">
    <div class="session-tab__bar">
      <span class="u-text--muted">Commits and diff against the base branch</span>
      <button class="p-button--base is-dense" type="button" :disabled="loading" @click="load">{{ loading ? 'Loading…' : 'Refresh' }}</button>
    </div>
    <div class="session-tab__body session-tab__body--scroll">
      <p v-if="error" class="u-text--muted">{{ error }}</p>
      <template v-else-if="changes">
        <ul v-if="changes.commits.length" class="code-commits">
          <li v-for="commit in changes.commits" :key="commit"><code>{{ commit }}</code></li>
        </ul>
        <p v-if="!changes.diff.trim()" class="u-text--muted">No changes yet.</p>
        <pre v-else class="code-diff"><span v-for="(line, i) in diffLines" :key="i" :class="line.kind">{{ line.text }}
</span></pre>
      </template>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import { getSessionChanges } from '../../lib/code-api.js';
import { classifyDiffLines } from '../../lib/diff-lines.js';
import { useLayoutWorkspace } from '../../lib/layout-workspace.js';

defineProps({ tab: { type: Object, required: true } });

const workspace = useLayoutWorkspace();
const changes = ref(null);
const error = ref('');
const loading = ref(false);
const diffLines = computed(() => classifyDiffLines(changes.value?.diff ?? ''));

async function load() {
  loading.value = true;
  error.value = '';
  try {
    changes.value = await getSessionChanges(workspace.sessionId);
  } catch (e) {
    error.value = e.message;
  } finally {
    loading.value = false;
  }
}

onMounted(load);
</script>

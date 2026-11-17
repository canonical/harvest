<template>
  <div class="session-tab">
    <div class="session-tab__bar">
      <span class="changes-summary">{{ summary }}</span>
      <button class="p-button--base is-dense has-icon" type="button" aria-label="Refresh changes" title="Refresh" :disabled="loading" @click="load">
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><polyline points="21 3 21 8 16 8"/></svg>
      </button>
    </div>
    <div class="session-tab__body session-tab__body--scroll">
      <p v-if="error" class="u-text--muted">{{ error }}</p>
      <template v-else-if="changes">
        <select
          v-if="changes.commits.length"
          v-model="selectedCommit"
          class="changes-commit-picker"
          aria-label="Show changes from"
          @change="load"
        >
          <option value="">All changes against the base branch</option>
          <option v-for="commit in changes.commits" :key="commit" :value="hashOf(commit)">{{ commit }}</option>
        </select>
        <p v-if="!files.length" class="u-text--muted">No changes yet.</p>
        <details v-for="(file, index) in files" :key="file.path" class="diff-file-block" :open="index < OPEN_FILES">
          <summary>
            <span class="diff-file-block__path">{{ file.path }}</span>
            <span class="diff-file-block__stats"><span class="diff-add">+{{ file.added }}</span> <span class="diff-del">−{{ file.removed }}</span></span>
          </summary>
          <pre class="code-diff"><span v-for="(line, i) in file.lines" :key="i" :class="line.kind">{{ line.text }}
</span></pre>
        </details>
      </template>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { getSessionChanges } from '../../lib/code-api.js';
import { diffFiles, diffSummary } from '../../lib/diff-lines.js';
import { useLayoutWorkspace } from '../../lib/layout-workspace.js';

defineProps({
  tab: { type: Object, required: true },
  paneId: { type: String, required: true },
});

const OPEN_FILES = 3;
const workspace = useLayoutWorkspace();
const changes = ref(null);
const error = ref('');
const loading = ref(false);
const selectedCommit = ref('');
const files = computed(() => diffFiles(changes.value?.diff ?? ''));
const shownCommits = computed(() => {
  const commits = changes.value?.commits ?? [];
  return selectedCommit.value ? commits.filter(c => hashOf(c) === selectedCommit.value) : commits;
});
const summary = computed(() => (changes.value ? diffSummary(shownCommits.value, files.value) : 'Changes against the base branch'));

function hashOf(commitLine) {
  return commitLine.split(' ')[0];
}

async function load() {
  loading.value = true;
  error.value = '';
  try {
    const next = await getSessionChanges(workspace.sessionId, selectedCommit.value || null);
    if (selectedCommit.value && !next.commits.some(c => hashOf(c) === selectedCommit.value)) {
      selectedCommit.value = '';
      changes.value = await getSessionChanges(workspace.sessionId);
      return;
    }
    changes.value = next;
  } catch (e) {
    error.value = e.message;
  } finally {
    loading.value = false;
  }
}

function loadWhenVisible() {
  if (document.visibilityState === 'visible') load();
}

watch(() => workspace.turnsFinished, load);
onMounted(() => {
  load();
  document.addEventListener('visibilitychange', loadWhenVisible);
});
onUnmounted(() => document.removeEventListener('visibilitychange', loadWhenVisible));
</script>

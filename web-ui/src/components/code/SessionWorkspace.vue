<template>
  <div class="session-workspace">
    <PaneSplit v-if="workspace.tree" :node="workspace.tree.root" />
    <p v-else-if="error" class="code-error" role="alert">{{ error }}</p>
  </div>
</template>

<script setup>
import { onMounted, onUnmounted, ref, watch } from 'vue';
import PaneSplit from '../layout/PaneSplit.vue';
import SessionTab from './SessionTab.vue';
import SessionTabMenu from './SessionTabMenu.vue';
import { provideLayoutWorkspace } from '../../lib/layout-workspace.js';
import { useSessionWorkspace } from '../../composables/useSessionWorkspace.js';

const props = defineProps({
  sessionId: { type: String, required: true },
  details: { type: Object, required: true },
});

const workspace = useSessionWorkspace(props.sessionId, { content: SessionTab, tabMenu: SessionTabMenu });
const error = ref('');
provideLayoutWorkspace(workspace);
defineExpose({ showJobs: () => workspace.showJobs() });

watch(() => props.details, (details) => workspace.setLists(details));

function flushWhenHidden() {
  if (document.visibilityState === 'hidden') workspace.flush();
}

onMounted(async () => {
  try {
    await workspace.load(props.details);
  } catch (e) {
    error.value = e.message;
  }
  document.addEventListener('visibilitychange', flushWhenHidden);
  window.addEventListener('beforeunload', workspace.flush);
});

onUnmounted(() => {
  workspace.flush();
  document.removeEventListener('visibilitychange', flushWhenHidden);
  window.removeEventListener('beforeunload', workspace.flush);
});
</script>

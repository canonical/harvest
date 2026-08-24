<template>
  <div class="design-view">
    <div v-if="loading" data-testid="design-loading">
      <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
    </div>

    <template v-else-if="deployment">
      <div class="design-view-header">
        <h2>{{ deployment.name }}</h2>
      </div>
      <DesignPanel
        :project-id="projectId"
        :deployment="deployment"
        @refresh="load"
      />
    </template>

    <div v-else class="design-view-error">Failed to load deployment.</div>
  </div>
</template>

<script setup>
import { ref, watch } from 'vue';
import DesignPanel from '../components/deployment/DesignPanel.vue';
import { getProjectDeploymentSingle } from '../lib/api.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const deployment = ref(null);
const loading    = ref(false);

async function load() {
  if (!props.projectId) return;
  loading.value = true;
  try {
    deployment.value = await getProjectDeploymentSingle(props.projectId);
  } catch {
    deployment.value = null;
  }
  loading.value = false;
}

watch(() => props.projectId, () => load(), { immediate: true });
</script>

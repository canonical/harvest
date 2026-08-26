<template>
  <div class="design-view">
    <div v-if="loading" data-testid="design-loading">
      <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
    </div>

    <template v-else-if="deployment">
      <div class="design-view-header">
        <p class="design-view__eyebrow" data-testid="design-eyebrow">Design</p>
        <div class="design-view__title-row">
          <h2 class="design-view__title">{{ deployment.name }}</h2>
          <span v-if="deployment.template" class="design-view__template-chip" data-testid="design-template-chip">
            {{ deployment.template.name }}
          </span>
        </div>
      </div>
      <DesignSetupPanel
        v-if="!deployment.design_doc"
        :project-id="projectId"
        :deployment-id="deployment.id"
        :group-id="groupId"
        @refresh="load"
      />
      <DesignPanel
        v-else
        :project-id="projectId"
        :deployment="deployment"
        @refresh="load"
      />
    </template>

    <div v-else class="design-view-error">Failed to load deployment.</div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import DesignPanel from '../components/deployment/DesignPanel.vue';
import DesignSetupPanel from '../components/deployment/DesignSetupPanel.vue';
import { getProjectDeploymentSingle } from '../lib/api.js';
import { useProjectStore } from '../stores/project.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const projectStore = useProjectStore();

const deployment = ref(null);
const loading    = ref(false);

const groupId = computed(() => projectStore.selectedProject?.group_id ?? null);

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
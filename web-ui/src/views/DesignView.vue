<template>
  <div class="design-view">
    <div v-if="loading" data-testid="design-loading">
      <LoadingSpinner text="Loading…" />
    </div>

    <template v-else-if="deployment">
      <div v-if="generating || !deployment.design_doc" class="design-view-header">
        <p class="p-text--small-caps u-text--muted" data-testid="design-eyebrow">Design</p>
        <div class="design-view__title-row">
          <h2 class="p-heading--3">{{ deployment.name }}</h2>
          <span v-if="deployment.template" class="p-chip" data-testid="design-template-chip">{{ deployment.template.name }}</span>
        </div>
      </div>
      <DesignGenerationPanel
        v-if="generating"
        :project-id="projectId"
        :deployment-id="deployment.id"
        :deployment-name="deployment.name"
        :body="generateBody"
        @done="onGenerationDone"
      />
      <DesignSetupPanel
        v-else-if="!deployment.design_doc"
        :project-id="projectId"
        :deployment-id="deployment.id"
        :group-id="groupId"
        @generate="onGenerate"
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
import DesignGenerationPanel from '../components/deployment/DesignGenerationPanel.vue';
import LoadingSpinner from '../components/deployment/LoadingSpinner.vue';
import { getProjectDeploymentSingle } from '../lib/api.js';
import { useProjectStore } from '../stores/project.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const projectStore = useProjectStore();

const deployment    = ref(null);
const loading       = ref(false);
const generating    = ref(false);
const generateBody  = ref({});

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

function onGenerate(body) {
  generateBody.value = body;
  generating.value = true;
}

async function onGenerationDone() {
  generating.value = false;
  generateBody.value = {};
  await load();
}

watch(() => props.projectId, () => load(), { immediate: true });
</script>

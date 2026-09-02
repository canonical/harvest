<template>
  <div class="deploy-view">
    <div v-if="loading" data-testid="deploy-loading">
      <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
    </div>

    <template v-else-if="deployment">
      <div v-if="!deployment.design_doc" class="deploy-view__needs-design" data-testid="deploy-needs-design">
        <h2>Generate a design first</h2>
        <p>
          A deployment design document is required before you can generate deployment artifacts.
          <router-link to="/design" data-testid="deploy-go-to-design">Go to the Design page →</router-link>
        </p>
      </div>

      <DeployGenerationPanel
        v-else-if="generating"
        :project-id="projectId"
        :deployment-id="deployment.id"
        :deployment-name="deployment.name"
        @done="onGenerationDone"
      />

      <template v-else-if="deployment.terraform_bundle">
        <div class="deploy-view-header">
          <p class="deploy-view__eyebrow" data-testid="deploy-eyebrow">Deploy</p>
          <div class="deploy-view__title-row">
            <h2 class="deploy-view__title">{{ deployment.name }}</h2>
          </div>
        </div>

        <div
          v-if="isBroken"
          class="p-notification--caution deploy-broken-banner"
          data-testid="broken-banner"
        >
          <div class="p-notification__content">
            <p class="p-notification__message">
              This deployment is broken.
            </p>
          </div>
        </div>

        <DeployArtifacts
          :project-id="projectId"
          :deployment="deployment"
          :agents="agents"
          @refresh="load"
        />
      </template>

      <DeployAgentsPanel
        v-else
        :project-id="projectId"
        :agents="agents"
        :reload="loadAgents"
        @next="onNext"
      />
    </template>

    <div v-else class="deploy-view-error">Failed to load deployment.</div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onUnmounted } from 'vue';
import DeployArtifacts from '../components/deployment/DeployArtifacts.vue';
import DeployAgentsPanel from '../components/deployment/DeployAgentsPanel.vue';
import DeployGenerationPanel from '../components/deployment/DeployGenerationPanel.vue';
import {
  getProjectDeploymentSingle, listProjectAgents,
  openProjectEvents,
} from '../lib/api.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const deployment    = ref(null);
const agents        = ref([]);
const loading       = ref(false);
const generating    = ref(false);
let eventSource     = null;

const isBroken = computed(() => ['broken', 'destroy_failed'].includes(deployment.value?.infra_state));

async function loadAgents() {
  if (!props.projectId) return;
  try {
    agents.value = await listProjectAgents(props.projectId);
  } catch {
    agents.value = [];
  }
}

async function load() {
  if (!props.projectId) return;
  loading.value = true;
  try {
    const d = await getProjectDeploymentSingle(props.projectId);
    deployment.value = d;
    agents.value = await listProjectAgents(props.projectId).catch(() => []);
  } catch {
    deployment.value = null;
  }
  loading.value = false;
}

function onNext() {
  generating.value = true;
}

async function onGenerationDone() {
  generating.value = false;
  await load();
}

function handleProjectEvent(e) {
  if (!deployment.value || e.deployment_id !== deployment.value.id) return;
  if (e.type === 'done') {
    load();
  }
}

onMounted(() => {
  if (props.projectId) {
    eventSource = openProjectEvents(props.projectId, null, handleProjectEvent);
  }
});

onUnmounted(() => {
  eventSource?.close();
});

watch(() => props.projectId, () => load(), { immediate: true });
</script>

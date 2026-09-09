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
        @cancel="onGenerationCancel"
      />

      <template v-else-if="deployment.terraform_bundle">
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
        @modal-state-change="onModalStateChange"
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
let agentPollTimer  = null;

const AGENT_POLL_INTERVAL_MS = 15_000;
const AGENT_POLL_FAST_MS     = 1_000;

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

function restartAgentPolling(ms) {
  clearInterval(agentPollTimer);
  agentPollTimer = setInterval(loadAgents, ms);
}

function onModalStateChange(open) {
  restartAgentPolling(open ? AGENT_POLL_FAST_MS : AGENT_POLL_INTERVAL_MS);
}

async function onGenerationDone() {
  generating.value = false;
  await load();
}

function onGenerationCancel() {
  generating.value = false;
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
    agentPollTimer = setInterval(loadAgents, AGENT_POLL_INTERVAL_MS);
  }
});

onUnmounted(() => {
  eventSource?.close();
  clearInterval(agentPollTimer);
});

watch(() => props.projectId, () => load(), { immediate: true });
</script>

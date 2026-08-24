<template>
  <div class="deploy-view">
    <div v-if="loading" data-testid="deploy-loading">
      <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
    </div>

    <template v-else-if="deployment">
      <div class="deploy-view-header">
        <h2>{{ deployment.name }}</h2>
        <span class="infra-state-badge" :class="infraStateClass(deployment.infra_state)">
          {{ infraStateLabel(deployment.infra_state) }}
        </span>
      </div>

      <div
        v-if="isBroken"
        class="p-notification--caution deploy-broken-banner"
        data-testid="broken-banner"
      >
        <div class="p-notification__content">
          <p class="p-notification__message">
            This deployment is broken —
            <router-link to="/change-requests?status=open" data-testid="view-change-requests-link">View change requests</router-link>
          </p>
        </div>
      </div>

      <ArtifactsPanel
        :project-id="projectId"
        :deployment="deployment"
        :runs="runs"
        :agents="agents"
        @refresh="load"
      />

      <div class="deploy-view__history">
        <h3>Run history</h3>
        <RunHistory
          :runs="runs"
          :live-entry="liveEntry"
          :live-log="runLog"
        />
      </div>
    </template>

    <div v-else class="deploy-view-error">Failed to load deployment.</div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onUnmounted } from 'vue';
import ArtifactsPanel from '../components/deployment/ArtifactsPanel.vue';
import RunHistory from '../components/deployment/RunHistory.vue';
import { getProjectDeploymentSingle, listDeploymentRuns, listProjectAgents, openProjectEvents } from '../lib/api.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const deployment = ref(null);
const runs       = ref([]);
const agents     = ref([]);
const loading    = ref(false);
const liveEntry  = ref(null);
const runLog     = ref([]);
let eventSource  = null;

const isBroken = computed(() => ['broken', 'destroy_failed'].includes(deployment.value?.infra_state));

const INFRA_STATE_LABELS = {
  none: 'Not deployed', up: 'Up', broken: 'Broken', destroyed: 'Destroyed', destroy_failed: 'Destroy failed',
};

function infraStateLabel(state) { return INFRA_STATE_LABELS[state] ?? state; }
function infraStateClass(state) {
  if (state === 'up') return 'infra-state-badge--up';
  if (state === 'broken' || state === 'destroy_failed') return 'infra-state-badge--broken';
  if (state === 'destroyed') return 'infra-state-badge--destroyed';
  return 'infra-state-badge--none';
}

async function load() {
  if (!props.projectId) return;
  loading.value = true;
  try {
    const d = await getProjectDeploymentSingle(props.projectId);
    deployment.value = d;
    const [r, a] = await Promise.all([
      listDeploymentRuns(props.projectId, d.id).catch(() => []),
      listProjectAgents(props.projectId).catch(() => []),
    ]);
    runs.value   = r;
    agents.value = a;
  } catch {
    deployment.value = null;
  }
  loading.value = false;
}

watch(() => props.projectId, () => load(), { immediate: true });
</script>

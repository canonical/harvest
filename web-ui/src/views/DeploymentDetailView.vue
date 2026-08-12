<template>
  <div class="deployment-detail-page">
    <div v-if="loading" class="deployment-detail-loading">
      <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
    </div>

    <template v-else-if="deployment">
      <div class="deployment-detail-header">
        <h2>{{ deployment.name }}</h2>
        <span class="infra-state-badge" :class="infraStateClass(deployment.infra_state)">
          {{ infraStateLabel(deployment.infra_state) }}
        </span>
        <span v-if="deployment.template" class="deployment-detail-header__template">
          Based on <strong>{{ deployment.template.name }}</strong>
        </span>
      </div>

      <nav class="deployment-phase-tabs">
        <button
          v-for="phase in phases"
          :key="phase.id"
          class="deployment-phase-tab"
          :class="{
            'deployment-phase-tab--active': selectedPhase === phase.id,
            'deployment-phase-tab--done':   phase.done,
          }"
          :disabled="phase.inert"
          type="button"
          @click="selectedPhase = phase.id"
        >
          <span class="deployment-phase-tab__marker">{{ phase.done ? '✓' : '' }}</span>
          {{ phase.label }}
        </button>
      </nav>

      <div class="deployment-phase-content">
        <EnvironmentPhase
          v-if="selectedPhase === 'environment'"
          :project-id="projectId"
          :deployment="deployment"
          @refresh="load"
        />
        <DesignPhase
          v-else-if="selectedPhase === 'design'"
          :project-id="projectId"
          :deployment="deployment"
          @refresh="load"
        />
        <ProvisionPhase
          v-else-if="selectedPhase === 'provision'"
          :project-id="projectId"
          :deployment="deployment"
          :runs="runs"
          :agents="agents"
          @refresh="load"
        />
      </div>
    </template>

    <div v-else class="deployment-detail-error">Failed to load deployment.</div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { useRoute } from 'vue-router';
import EnvironmentPhase from '../components/deployment/EnvironmentPhase.vue';
import DesignPhase from '../components/deployment/DesignPhase.vue';
import ProvisionPhase from '../components/deployment/ProvisionPhase.vue';
import { getProjectDeployment, listDeploymentRuns, listProjectAgents } from '../lib/api.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const route = useRoute();

const deployment    = ref(null);
const runs           = ref([]);
const agents         = ref([]);
const loading        = ref(false);
const selectedPhase  = ref('environment');
let phaseInitialized = false;

const deploymentId = computed(() => route.params.id);

const INFRA_STATE_LABELS = {
  none: 'Not deployed', up: 'Up', broken: 'Broken', destroyed: 'Destroyed', destroy_failed: 'Destroy failed',
};

function infraStateLabel(state) {
  return INFRA_STATE_LABELS[state] ?? state;
}

function infraStateClass(state) {
  if (state === 'up') return 'infra-state-badge--up';
  if (state === 'broken' || state === 'destroy_failed') return 'infra-state-badge--broken';
  if (state === 'destroyed') return 'infra-state-badge--destroyed';
  return 'infra-state-badge--none';
}

const phases = computed(() => {
  const d = deployment.value;
  if (!d) return [];
  const everApplied = runs.value.some(r => r.action === 'apply' && r.status === 'success');
  return [
    { id: 'environment', label: 'Describe environment', done: !!d.environment_description?.trim(), inert: false },
    { id: 'design',      label: 'Design',                done: !!d.design_doc,                      inert: false },
    { id: 'provision',   label: 'Deploy',                 done: everApplied,                          inert: false },
    { id: 'validate',    label: 'Validate',                done: false,                                inert: true },
    { id: 'guide',       label: 'Guide',                    done: false,                                inert: true },
  ];
});

function firstIncompletePhase() {
  const found = phases.value.find(p => !p.inert && !p.done);
  return found ? found.id : 'environment';
}

async function load() {
  if (!props.projectId || !deploymentId.value) return;
  loading.value = true;
  try {
    const [d, r, a] = await Promise.all([
      getProjectDeployment(props.projectId, deploymentId.value),
      listDeploymentRuns(props.projectId, deploymentId.value).catch(() => []),
      listProjectAgents(props.projectId).catch(() => []),
    ]);
    deployment.value = d;
    runs.value        = r;
    agents.value       = a;
    if (!phaseInitialized) {
      selectedPhase.value = firstIncompletePhase();
      phaseInitialized     = true;
    }
  } catch {
    deployment.value = null;
  }
  loading.value = false;
}

watch(deploymentId, () => {
  phaseInitialized = false;
  load();
}, { immediate: true });
</script>

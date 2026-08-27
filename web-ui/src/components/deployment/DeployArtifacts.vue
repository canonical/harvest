<template>
  <div class="deploy-artifacts" data-testid="deploy-artifacts">
    <div class="deploy-artifacts__toolbar">
      <span class="infra-state-badge" :class="infraStateClass(deployment.infra_state)">
        {{ infraStateLabel(deployment.infra_state) }}
      </span>
      <div v-if="agents.length > 0" class="deploy-artifacts__agent">
        <label for="deploy-agent-select" class="deploy-artifacts__agent-label">Agent</label>
        <select id="deploy-agent-select" v-model="selectedAgentId" data-testid="agent-select">
          <option value="" disabled>Select an agent</option>
          <option v-for="a in agents" :key="a.id" :value="a.id">{{ a.hostname }}</option>
        </select>
      </div>
      <button
        class="p-button--positive is-dense"
        type="button"
        data-testid="run-all-btn"
        :disabled="!selectedAgentId || running"
        @click="runAll"
      >Run all</button>
    </div>

    <div class="deploy-artifacts__body">
      <div class="deploy-artifacts__dag">
        <DagView
          :plan="plan"
          :step-files="stepFiles"
          :step-status="stepStatus"
          @select-artifact="selectArtifact"
          @run-all="runAll"
          @run-node="runNode"
          @plan-preview="planPreview"
        />
      </div>

      <div class="deploy-artifacts__editor">
        <ArtifactEditor
          :project-id="projectId"
          :deployment-id="deployment.id"
          :artifact-id="selectedArtifactId"
          @saved="onSaved"
        />
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch, onMounted, onUnmounted } from 'vue';
import DagView from './DagView.vue';
import ArtifactEditor from './ArtifactEditor.vue';
import {
  getExecutionPlan, getArtifact, runDag, openProjectEvents,
} from '../../lib/api.js';

const props = defineProps({
  projectId:  { type: String, required: true },
  deployment: { type: Object, required: true },
  agents:     { type: Array, default: () => [] },
});
const emit = defineEmits(['refresh']);

const plan             = ref({ deploy_steps: [], destroy_steps: [] });
const stepFiles        = ref({});
const stepStatus       = ref({});
const selectedArtifactId = ref(null);
const selectedAgentId  = ref('');
const running          = ref(false);
let eventSource        = null;

watch(() => props.agents, (list) => {
  if (!selectedAgentId.value && list.length === 1) selectedAgentId.value = list[0].id;
}, { immediate: true });

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

async function loadPlan() {
  try {
    plan.value = await getExecutionPlan(props.projectId, props.deployment.id);
    for (const step of [...plan.value.deploy_steps, ...plan.value.destroy_steps]) {
      if (step.artifact?.kind === 'terraform' || step.artifact?.kind === 'terragrunt') {
        loadStepFiles(step);
      }
    }
  } catch {
    plan.value = { deploy_steps: [], destroy_steps: [] };
  }
}

async function loadStepFiles(step) {
  try {
    const artifact = await getArtifact(step.artifact.id);
    stepFiles.value[step.id] = JSON.parse(artifact.content || '{}');
  } catch {
    stepFiles.value[step.id] = {};
  }
}

function selectArtifact(artifactId) {
  selectedArtifactId.value = artifactId;
}

async function runAll() {
  if (!selectedAgentId.value || running.value) return;
  running.value = true;
  try {
    await runDag(props.projectId, props.deployment.id, { agent_id: selectedAgentId.value, timeout_secs: 300 });
    emit('refresh');
  } catch {
  } finally {
    running.value = false;
  }
}

async function runNode(stepId) {
  if (!selectedAgentId.value || running.value) return;
  running.value = true;
  try {
    await runDag(props.projectId, props.deployment.id, { agent_id: selectedAgentId.value, timeout_secs: 300 });
    emit('refresh');
  } catch {
  } finally {
    running.value = false;
  }
}

function planPreview(stepId) {}

function onSaved() {
  emit('refresh');
}

function handleProjectEvent(e) {
  if (!props.deployment || e.deployment_id !== props.deployment.id) return;
  if (e.type === 'deployment_run_log') {
    if (e.step_id) stepStatus.value[e.step_id] = e.status;
  }
  if (e.type === 'done') {
    emit('refresh');
  }
}

onMounted(() => {
  loadPlan();
  if (props.projectId) {
    eventSource = openProjectEvents(props.projectId, null, handleProjectEvent);
  }
});

onUnmounted(() => {
  eventSource?.close();
});

watch(() => props.deployment.terraform_bundle?.id, () => {
  loadPlan();
});
</script>

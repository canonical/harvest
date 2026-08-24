<template>
  <div class="artifacts-panel">
    <div class="artifacts-panel__toolbar">
      <div v-if="agents.length > 0" class="form-group artifacts-panel__agent">
        <label for="artifacts-agent-select">Agent</label>
        <select id="artifacts-agent-select" v-model="selectedAgentId" data-testid="agent-select">
          <option value="" disabled>Select an agent</option>
          <option v-for="a in agents" :key="a.id" :value="a.id">{{ a.hostname }}</option>
        </select>
      </div>
      <nav class="artifacts-panel__tabs">
        <button
          type="button"
          class="artifacts-panel__tab"
          :class="{ 'artifacts-panel__tab--active': tab === 'dag' }"
          @click="tab = 'dag'"
        >DAG</button>
        <button
          type="button"
          class="artifacts-panel__tab"
          data-testid="run-history-tab"
          :class="{ 'artifacts-panel__tab--active': tab === 'history' }"
          @click="tab = 'history'"
        >Run history</button>
      </nav>
      <BusyStatus v-if="busyLabel" :text="busyLabel" />
    </div>

    <template v-if="!deployment.terraform_bundle">
      <div class="artifacts-panel__generate-area">
        <button
          class="p-button--positive is-dense"
          data-testid="generate-artifacts-btn"
          type="button"
          :disabled="generating"
          @click="generate"
        >{{ generating ? 'Generating…' : 'Generate deployment artifacts' }}</button>
      </div>
    </template>

    <template v-else>
      <div
        v-if="hasUncoveredApply"
        class="p-notification--caution"
        data-testid="coverage-warning"
      >
        <div class="p-notification__content">
          <p class="p-notification__message">
            One or more terraform apply steps have no matching destroy step.
          </p>
        </div>
      </div>

      <div class="artifacts-panel__body">
        <DagView
          v-if="tab === 'dag'"
          :plan="plan"
          :step-files="stepFiles"
          :step-status="stepStatus"
          @run-all="runAll"
          @run-node="runNode"
          @plan-preview="planPreview"
        />
        <RunHistory
          v-else
          :runs="runs"
          :live-entry="liveEntry"
          :live-log="runLog"
        />
      </div>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onUnmounted } from 'vue';
import DagView from './DagView.vue';
import RunHistory from './RunHistory.vue';
import BusyStatus from './BusyStatus.vue';
import {
  getExecutionPlan, runDag, getArtifact, generateProvision, openProjectEvents,
} from '../../lib/api.js';

const props = defineProps({
  projectId:  { type: String, required: true },
  deployment: { type: Object, required: true },
  runs:       { type: Array, default: () => [] },
  agents:     { type: Array, default: () => [] },
});
const emit = defineEmits(['refresh']);

const tab              = ref('dag');
const plan             = ref({ deploy_steps: [], destroy_steps: [] });
const stepFiles        = ref({});
const stepStatus       = ref({});
const generating       = ref(false);
const running          = ref(false);
const runLog           = ref([]);
const liveEntry        = ref(null);
const selectedAgentId   = ref('');

watch(() => props.agents, (list) => {
  if (!selectedAgentId.value && list.length === 1) selectedAgentId.value = list[0].id;
}, { immediate: true });

const busyLabel = computed(() => {
  if (generating.value) return 'Generating deployment artifacts…';
  if (running.value)    return 'Running DAG…';
  return null;
});

const hasUncoveredApply = computed(() => {
  const applyArtifacts = new Set(
    (plan.value.deploy_steps ?? [])
      .filter(s => s.action === 'apply')
      .map(s => s.artifact?.id)
      .filter(Boolean)
  );
  const destroyArtifacts = new Set(
    (plan.value.destroy_steps ?? [])
      .filter(s => s.action === 'destroy')
      .map(s => s.artifact?.id)
      .filter(Boolean)
  );
  for (const id of applyArtifacts) {
    if (!destroyArtifacts.has(id)) return true;
  }
  return false;
});

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

async function generate() {
  generating.value = true;
  try {
    await generateProvision(props.projectId, props.deployment.id);
    emit('refresh');
  } finally {
    generating.value = false;
  }
}

async function runAll() {
  if (!selectedAgentId.value || running.value) return;
  running.value = true;
  runLog.value = [];
  liveEntry.value = { action: 'deploy', agentHostname: props.agents.find(a => a.id === selectedAgentId.value)?.hostname };
  tab.value = 'history';
  try {
    await runDag(props.projectId, props.deployment.id, { agent_id: selectedAgentId.value, timeout_secs: 300 });
  } catch (e) {
    runLog.value.push({ stream: 'stderr', line: e.message || 'Run failed' });
  } finally {
    running.value = false;
    liveEntry.value = null;
    emit('refresh');
  }
}

async function runNode(stepId) {
  if (!selectedAgentId.value || running.value) return;
  running.value = true;
  runLog.value = [];
  liveEntry.value = { action: 'run', agentHostname: props.agents.find(a => a.id === selectedAgentId.value)?.hostname };
  tab.value = 'history';
  try {
    await runDag(props.projectId, props.deployment.id, { agent_id: selectedAgentId.value, timeout_secs: 300 });
  } catch (e) {
    runLog.value.push({ stream: 'stderr', line: e.message || 'Run failed' });
  } finally {
    running.value = false;
    liveEntry.value = null;
    emit('refresh');
  }
}

function planPreview(stepId) {
}

let eventSource = null;

function handleProjectEvent(e) {
  if (e.deployment_id !== props.deployment.id) return;
  if (e.type === 'deployment_run_log') {
    runLog.value.push({ stream: e.stream, line: e.line });
    if (runLog.value.length > 2000) runLog.value.shift();
  }
  if (e.type === 'done') {
    emit('refresh');
  }
}

onMounted(() => {
  loadPlan();
  eventSource = openProjectEvents(props.projectId, null, handleProjectEvent);
});

onUnmounted(() => {
  eventSource?.close();
});

watch(() => props.deployment.terraform_bundle?.id, () => {
  loadPlan();
});
</script>

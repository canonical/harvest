<template>
  <div class="provision-phase">
    <div class="provision-phase__toolbar">
      <div v-if="agents.length > 1" class="form-group provision-phase__agent-select">
        <label for="provision-agent-select">Agent</label>
        <select id="provision-agent-select" v-model="selectedAgentId" data-testid="provision-agent-select">
          <option value="" disabled>Select an agent</option>
          <option v-for="a in agents" :key="a.id" :value="a.id">{{ a.hostname }}</option>
        </select>
      </div>
      <button
        data-testid="deploy-btn"
        class="p-button--positive is-dense"
        type="button"
        :disabled="!canDeploy || !deployment.terraform_bundle || !selectedAgentId || running"
        @click="runAction('deploy')"
      >Deploy</button>
      <button
        data-testid="redeploy-btn"
        class="p-button--positive is-dense"
        type="button"
        :disabled="!canRedeploy || !deployment.terraform_bundle || !selectedAgentId || running"
        @click="runAction('redeploy')"
      >Redeploy</button>
      <button
        data-testid="destroy-btn"
        class="p-button--negative is-dense"
        type="button"
        :disabled="!canDestroy || !deployment.terraform_bundle || !selectedAgentId || running"
        @click="runAction('destroy')"
      >Destroy</button>
    </div>

    <div class="provision-phase__body">
      <div class="provision-phase__left">
      <BusyStatus v-if="busyLabel" :text="busyLabel" />

      <ul v-if="generating && generationStatus.length" class="provision-generation-status" data-testid="generation-status">
        <li v-for="(line, i) in generationStatus" :key="i">{{ line.text }}</li>
      </ul>

      <ul v-if="diagnosing && diagnosisTrace.length" class="provision-generation-status" data-testid="diagnosis-trace">
        <li v-for="(line, i) in diagnosisTrace" :key="i">{{ line.text }}</li>
      </ul>

      <template v-if="!diagnosing && diagnosisFailed">
        <div class="p-notification--negative" data-testid="diagnosis-failed">
          <div class="p-notification__content"><p class="p-notification__message">{{ diagnosisError }}</p></div>
        </div>
        <button
          class="p-button--positive is-dense"
          data-testid="retry-diagnosis-btn"
          type="button"
          @click="retryDiagnosis"
        >Retry diagnosis</button>
      </template>

      <template v-if="!deployment.terraform_bundle">
        <div v-if="!generating && generateError" class="p-notification--negative">
          <div class="p-notification__content"><p class="p-notification__message">{{ generateError }}</p></div>
        </div>
        <button
          v-if="!generating && generateError"
          class="p-button--positive is-dense"
          data-testid="generate-provision-btn"
          type="button"
          @click="generate"
        >Retry</button>
      </template>

      <template v-else>
        <template v-if="pendingChange">
          <div class="provision-change-explanation">{{ pendingChange.explanation }}</div>
          <div v-if="changeError" class="p-notification--negative">
            <div class="p-notification__content"><p class="p-notification__message">{{ changeError }}</p></div>
          </div>
          <div class="modal-actions">
            <button data-testid="discard-change-btn" class="p-button--base is-dense" type="button" @click="discardChange">Discard</button>
            <button
              data-testid="approve-change-btn"
              class="p-button--positive is-dense"
              type="button"
              :disabled="applying"
              @click="approveChange"
            >{{ applying ? 'Applying…' : 'Apply' }}</button>
          </div>
          <div v-if="!pendingChange.fromDiagnosis" class="form-group">
            <label for="propose-again-instructions">Propose something else instead</label>
            <textarea id="propose-again-instructions" v-model="instructions" rows="3"></textarea>
            <button class="p-button--base is-dense" type="button" :disabled="proposing" @click="proposeChange()">
              {{ proposing ? 'Working…' : 'Propose' }}
            </button>
          </div>
        </template>

        <template v-else-if="!isBroken && deployment.infra_state === 'up'">
          <div class="form-group">
            <label for="propose-change-instructions">Request a change</label>
            <textarea
              id="propose-change-instructions"
              data-testid="propose-change-instructions"
              v-model="instructions"
              rows="3"
              placeholder="Describe what you'd like to change about the infrastructure"
            ></textarea>
          </div>
          <button
            data-testid="propose-change-btn"
            class="p-button--base is-dense"
            type="button"
            :disabled="proposing || !instructions.trim()"
            @click="proposeChange()"
          >{{ proposing ? 'Working…' : 'Propose change' }}</button>
        </template>

        <div v-if="proposeError" class="p-notification--negative">
          <div class="p-notification__content"><p class="p-notification__message">{{ proposeError }}</p></div>
        </div>
      </template>
    </div>

    <div class="provision-phase__right">
      <nav class="provision-tabs">
        <button
          type="button"
          class="provision-tab"
          :class="{ 'provision-tab--active': rightTab === 'artifacts' }"
          @click="rightTab = 'artifacts'"
        >Artifacts</button>
        <button
          type="button"
          class="provision-tab"
          data-testid="run-history-tab"
          :class="{ 'provision-tab--active': rightTab === 'history' }"
          @click="rightTab = 'history'"
        >Run history</button>
      </nav>

      <div class="provision-tab-content">
        <template v-if="rightTab === 'artifacts'">
          <DiffView v-if="pendingChange" :before="pendingChange.current_files ?? bundleFiles" :after="pendingChange.proposed_files" />
          <div v-else class="provision-files">
            <div v-for="(content, path) in bundleFiles" :key="path" class="provision-files__file">
              <div class="provision-files__path">{{ path }}</div>
              <pre>{{ content }}</pre>
            </div>
          </div>
        </template>

        <RunHistory v-else :runs="runs" :live-entry="liveEntry" :live-log="runLog" />
      </div>
    </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onUnmounted } from 'vue';
import DiffView from './DiffView.vue';
import BusyStatus from './BusyStatus.vue';
import RunHistory from './RunHistory.vue';
import {
  getArtifact,
  generateProvision,
  proposeProvisionChange,
  applyProvisionChange,
  deployDeployment,
  redeployDeployment,
  destroyDeployment,
  openProjectEvents,
  diagnoseProvisionFailure,
  dismissProvisionDiagnosis,
} from '../../lib/api.js';

const MAX_RUN_LOG_LINES = 2000;
const RUNNING_LABELS = { deploy: 'Deploying', redeploy: 'Redeploying', destroy: 'Destroying' };
const ACTION_FNS = { deploy: deployDeployment, redeploy: redeployDeployment, destroy: destroyDeployment };
const TRACE_EVENT_TYPES = new Set(['thinking', 'thinking_delta', 'tool_call']);
const TOOL_LABELS = {
  generate_artifact:        'Generating the Terraform/Terragrunt bundle',
  link_deployment_artifact: 'Linking the generated bundle to this deployment',
};
const DIAGNOSIS_TOOL_LABELS = {
  read_provision_bundle:    'Reading the current Terraform/Terragrunt bundle',
  propose_provision_bundle: 'Proposing a corrected bundle',
  run_command:               'Running a diagnostic command',
  run_terraform_plan:        'Running terraform plan',
};

const props = defineProps({
  projectId:  { type: String, required: true },
  deployment: { type: Object, required: true },
  runs:       { type: Array, default: () => [] },
  agents:     { type: Array, default: () => [] },
});
const emit = defineEmits(['refresh']);

const generating       = ref(false);
const generateError    = ref(null);
const generationStatus = ref([]);
const bundleFiles      = ref({});
let autoGenerateAttempted = false;

const instructions   = ref('');
const proposing       = ref(false);
const proposeError    = ref(null);
const pendingChange   = ref(null);
const applying         = ref(false);
const changeError      = ref(null);

const diagnosisStarting = ref(false);
const diagnosisTrace    = ref([]);
let diagnosisRequestedForRunId = null;

const canDeploy   = computed(() => ['none', 'destroyed'].includes(props.deployment.infra_state));
const canRedeploy = computed(() => ['up', 'broken', 'destroy_failed'].includes(props.deployment.infra_state));
const canDestroy  = computed(() => !['none', 'destroyed'].includes(props.deployment.infra_state ?? 'none'));
const isBroken    = computed(() => ['broken', 'destroy_failed'].includes(props.deployment.infra_state));

const selectedAgentId = ref('');
const running          = ref(false);
const runningKind      = ref(null);
const runLog           = ref([]);
const rightTab         = ref('artifacts');
const liveEntry        = ref(null);

watch(() => props.agents, (list) => {
  if (!selectedAgentId.value && list.length === 1) selectedAgentId.value = list[0].id;
}, { immediate: true });

const diagnosing = computed(() => diagnosisStarting.value || props.deployment.diagnosis?.status === 'running');
const diagnosisFailed = computed(() => !diagnosing.value && props.deployment.diagnosis?.status === 'failed');
const diagnosisError  = computed(() => props.deployment.diagnosis?.error || 'Diagnosis failed.');
const latestFailedRun = computed(() => props.runs.find(r => r.status === 'failed'));

const busyLabel = computed(() => {
  if (generating.value) return 'Generating deployment artifacts…';
  if (diagnosing.value) return 'Diagnosing the failure…';
  if (running.value) {
    const hostname = props.agents.find(a => a.id === selectedAgentId.value)?.hostname;
    return `${RUNNING_LABELS[runningKind.value]}${hostname ? ` on ${hostname}` : ''}…`;
  }
  if (proposing.value)  return 'Proposing a change…';
  if (applying.value)   return 'Applying change…';
  return null;
});

async function loadBundleFiles() {
  if (!props.deployment.terraform_bundle) {
    bundleFiles.value = {};
    return;
  }
  try {
    const artifact = await getArtifact(props.deployment.terraform_bundle.id);
    bundleFiles.value = JSON.parse(artifact.content || '{}');
  } catch {
    bundleFiles.value = {};
  }
}

async function generate() {
  generating.value = true;
  generateError.value = null;
  generationStatus.value = [];
  try {
    await generateProvision(props.projectId, props.deployment.id);
    emit('refresh');
  } catch (e) {
    generateError.value = e.message || 'Failed to generate deployment artifacts';
  } finally {
    generating.value = false;
  }
}

function handleGenerationEvent(e) {
  if (e.type === 'tool_call') {
    generationStatus.value.push({ text: TOOL_LABELS[e.name] ?? e.name.replace(/_/g, ' ') });
    return;
  }
  const last = generationStatus.value.at(-1);
  if (e.type === 'thinking_delta' && last) {
    last.text += e.text ?? '';
  } else {
    generationStatus.value.push({ text: e.text ?? '' });
  }
}

async function proposeChange() {
  proposing.value = true;
  proposeError.value = null;
  try {
    pendingChange.value = await proposeProvisionChange(props.projectId, props.deployment.id, {
      instructions: instructions.value.trim(),
    });
    instructions.value = '';
  } catch (e) {
    proposeError.value = e.message || 'Failed to propose a change';
  } finally {
    proposing.value = false;
  }
}

async function discardChange() {
  const fromDiagnosis = pendingChange.value?.fromDiagnosis;
  pendingChange.value = null;
  changeError.value = null;
  if (fromDiagnosis) {
    try { await dismissProvisionDiagnosis(props.projectId, props.deployment.id); } catch {}
    emit('refresh');
  }
}

function handleDiagnosisEvent(e) {
  if (e.type === 'tool_call') {
    diagnosisTrace.value.push({ text: DIAGNOSIS_TOOL_LABELS[e.name] ?? e.name.replace(/_/g, ' ') });
    return;
  }
  const last = diagnosisTrace.value.at(-1);
  if (e.type === 'thinking_delta' && last) {
    last.text += e.text ?? '';
  } else {
    diagnosisTrace.value.push({ text: e.text ?? '' });
  }
}

async function startDiagnosis() {
  diagnosisTrace.value = [];
  diagnosisStarting.value = true;
  try {
    await diagnoseProvisionFailure(props.projectId, props.deployment.id);
  } catch {
    // the deployment simply keeps no diagnosis — the user can retry once a bundle exists
  } finally {
    diagnosisStarting.value = false;
  }
  emit('refresh');
}

function retryDiagnosis() {
  diagnosisRequestedForRunId = null;
  startDiagnosis();
}

async function approveChange() {
  if (!pendingChange.value) return;
  applying.value = true;
  changeError.value = null;
  const shouldRedeploy = isBroken.value && !!selectedAgentId.value;
  try {
    await applyProvisionChange(props.projectId, props.deployment.id, { files: pendingChange.value.proposed_files });
    pendingChange.value = null;
    await loadBundleFiles();
    emit('refresh');
    if (shouldRedeploy) await runAction('redeploy');
  } catch (e) {
    changeError.value = e.message || 'Failed to apply the change';
  } finally {
    applying.value = false;
  }
}

async function runAction(kind) {
  if (!selectedAgentId.value || running.value || !props.deployment.terraform_bundle) return;
  running.value = true;
  runningKind.value = kind;
  runLog.value = [];
  const agentHostname = props.agents.find(a => a.id === selectedAgentId.value)?.hostname;
  liveEntry.value = { action: kind, agentHostname };
  rightTab.value = 'history';
  try {
    await ACTION_FNS[kind](props.projectId, props.deployment.id, { agent_id: selectedAgentId.value });
  } catch {
    // failures are recorded server-side as runs — the UI reacts via refresh + infra_state
  } finally {
    running.value = false;
    runningKind.value = null;
    liveEntry.value = null;
    emit('refresh');
  }
}

let logEventSource = null;

function handleProjectEvent(e) {
  if (e.deployment_id !== props.deployment.id) return;
  if (e.type === 'deployment_run_log') {
    runLog.value.push({ stream: e.stream, line: e.line });
    if (runLog.value.length > MAX_RUN_LOG_LINES) runLog.value.shift();
    return;
  }
  if (e.type === 'done') {
    emit('refresh');
    return;
  }
  if (!TRACE_EVENT_TYPES.has(e.type)) return;
  if (generating.value) handleGenerationEvent(e);
  else if (diagnosing.value) handleDiagnosisEvent(e);
}

onMounted(() => {
  logEventSource = openProjectEvents(props.projectId, null, handleProjectEvent);
});

onUnmounted(() => {
  logEventSource?.close();
});

watch(() => props.deployment.terraform_bundle?.id, () => {
  pendingChange.value = null;
  loadBundleFiles();
}, { immediate: true });

// No bundle yet: generation happens automatically once, the moment we know that for sure.
watch(() => props.deployment.terraform_bundle, (bundle) => {
  if (bundle || autoGenerateAttempted || generating.value) return;
  autoGenerateAttempted = true;
  generate();
}, { immediate: true });

// A proposed fix lives in the Artifacts tab (as a diff) — jump back there once one is ready.
watch(pendingChange, (value) => {
  if (value) rightTab.value = 'artifacts';
});

watch(() => props.deployment.diagnosis, (diagnosis) => {
  if (diagnosis?.status === 'proposed') {
    pendingChange.value = { explanation: diagnosis.explanation, proposed_files: diagnosis.files, fromDiagnosis: true };
    changeError.value = null;
  }
}, { immediate: true });

watch(
  () => [isBroken.value, props.deployment.terraform_bundle?.id, latestFailedRun.value?.id, props.deployment.diagnosis?.status, props.deployment.diagnosis?.run_id],
  () => {
    if (!isBroken.value || !props.deployment.terraform_bundle) return;
    const runId = latestFailedRun.value?.id;
    if (!runId) return;
    if (props.deployment.diagnosis?.run_id === runId) return;
    if (diagnosisRequestedForRunId === runId) return;
    diagnosisRequestedForRunId = runId;
    startDiagnosis();
  },
  { immediate: true },
);
</script>

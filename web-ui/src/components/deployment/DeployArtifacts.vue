<template>
  <div class="deploy-artifacts" data-testid="deploy-artifacts">
    <div class="deploy-artifacts__status-row">
      <span class="infra-state-badge" :class="infraStateClass(deployment.infra_state)" data-testid="infra-state-badge">
        {{ infraStateLabel(deployment.infra_state) }}
      </span>
    </div>

    <div class="deploy-artifacts__body">
      <aside class="deploy-artifacts__sidebar" data-testid="deploy-artifacts-sidebar">
        <div class="deploy-artifacts__sidebar-header">
          <h3>Artifacts</h3>
          <button
            class="p-button--positive is-dense"
            type="button"
            data-testid="add-artifact-btn"
            @click="addArtifactOpen = true"
          >Add artifact</button>
        </div>
        <ul class="deploy-artifacts__list">
          <li
            v-for="item in uniqueArtifacts"
            :key="item.artifactId"
            class="deploy-artifacts__item"
            :class="{ 'deploy-artifacts__item--active': selectedArtifactId === item.artifactId }"
            :data-testid="`artifact-item-${item.artifactId}`"
            @click="selectArtifact(item.artifactId)"
          >
            <span class="artifact-kind-badge" :class="kindBadgeClass(item.kind)">{{ kindLabel(item.kind) }}</span>
            <div class="deploy-artifacts__item-text">
              <span class="deploy-artifacts__item-title">{{ item.title }}</span>
              <span class="deploy-artifacts__item-meta">{{ item.action }}</span>
            </div>
          </li>
        </ul>
        <div v-if="uniqueArtifacts.length === 0" class="deploy-artifacts__sidebar-empty">
          <p>No artifacts in the execution plan.</p>
        </div>
      </aside>

      <div class="deploy-artifacts__editor">
        <ArtifactEditor
          :project-id="projectId"
          :deployment-id="deployment.id"
          :artifact-id="selectedArtifactId"
          @saved="onSaved"
        />
      </div>
    </div>

    <AddArtifactModal
      :open="addArtifactOpen"
      :project-id="projectId"
      :deployment-id="deployment.id"
      @close="addArtifactOpen = false"
      @added="onArtifactAdded"
    />
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onUnmounted } from 'vue';
import ArtifactEditor from './ArtifactEditor.vue';
import AddArtifactModal from './AddArtifactModal.vue';
import {
  getExecutionPlan, setExecutionPlan, openProjectEvents,
} from '../../lib/api.js';

const props = defineProps({
  projectId:  { type: String, required: true },
  deployment: { type: Object, required: true },
  agents:     { type: Array, default: () => [] },
});
const emit = defineEmits(['refresh']);

const plan                 = ref({ deploy_steps: [], destroy_steps: [] });
const selectedArtifactId   = ref(null);
const addArtifactOpen       = ref(false);

let eventSource = null;

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

function kindLabel(kind) {
  if (kind === 'pdf') return 'PDF';
  if (kind === 'terraform') return 'Terraform';
  if (kind === 'terragrunt') return 'Terragrunt';
  if (kind === 'bash') return 'Bash';
  return 'Markdown';
}

function kindBadgeClass(kind) {
  if (kind === 'pdf') return 'artifact-kind-badge--pdf';
  if (kind === 'terraform' || kind === 'terragrunt') return 'artifact-kind-badge--terraform';
  if (kind === 'bash') return 'artifact-kind-badge--bash';
  return 'artifact-kind-badge--markdown';
}

const uniqueArtifacts = computed(() => {
  const seen = new Set();
  const items = [];
  for (const step of [...plan.value.deploy_steps, ...plan.value.destroy_steps]) {
    if (!step.artifact) continue;
    if (seen.has(step.artifact.id)) continue;
    seen.add(step.artifact.id);
    items.push({
      artifactId: step.artifact.id,
      kind:       step.artifact.kind,
      title:      step.artifact.title,
      action:     step.action,
    });
  }
  return items;
});

async function loadPlan() {
  try {
    plan.value = await getExecutionPlan(props.projectId, props.deployment.id);
  } catch {
    plan.value = { deploy_steps: [], destroy_steps: [] };
  }
}

function selectArtifact(artifactId) {
  selectedArtifactId.value = artifactId;
}

function onSaved() {
}

async function onArtifactAdded(newArtifact) {
  addArtifactOpen.value = false;
  const action = newArtifact.kind === 'terraform' || newArtifact.kind === 'terragrunt' ? 'apply' : 'run';
  const deploySteps = plan.value.deploy_steps.map(s => ({
    artifact_id: s.artifact.id,
    action:      s.action,
    label:       s.label,
    depends_on:  (s.depends_on ?? []).map(depId => {
      const idx = plan.value.deploy_steps.findIndex(ds => ds.id === depId);
      return idx >= 0 ? idx : 0;
    }),
  }));
  deploySteps.push({
    artifact_id: newArtifact.id,
    action,
    label: newArtifact.id,
    depends_on: deploySteps.length > 0 ? [deploySteps.length - 1] : [],
  });
  try {
    await setExecutionPlan(props.projectId, props.deployment.id, {
      deploy_steps: deploySteps,
      destroy_steps: [],
    });
  } catch {}
  await loadPlan();
  selectedArtifactId.value = newArtifact.id;
}

function handleProjectEvent(e) {
  if (!props.deployment || e.deployment_id !== props.deployment.id) return;
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

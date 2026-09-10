<template>
  <div class="deploy-artifacts" data-testid="deploy-artifacts">
    <div class="deploy-artifacts__status-row">
      <span class="infra-state-badge" :class="infraStateClass(deployment.infra_state)" data-testid="infra-state-badge">
        {{ infraStateLabel(deployment.infra_state) }}
      </span>
      <div class="deploy-artifacts__top-actions">
        <button
          class="p-button--brand is-dense"
          type="button"
          data-testid="propose-change-btn"
          @click="openProposeModal"
        >Propose a change</button>
        <button
          class="p-button--positive is-dense"
          type="button"
          data-testid="add-artifact-btn"
          @click="addArtifactOpen = true"
        >Add artifact</button>
      </div>
    </div>

    <div v-if="proposalProposing" class="deploy-artifacts__proposing" data-testid="deploy-artifacts-proposing">
      <DesignGenerationPanel
        :project-id="projectId"
        :deployment-id="deployment.id"
        :stream-fn="proposeProvisionChangeStream"
        :body="proposalBody"
        preparing-text="Preparing proposed changes…"
        ready-text="Proposed changes ready"
        failed-text="Failed to propose changes"
        @done="onProposalStreamDone"
        @cancel="onProposalStreamCancel"
      />
    </div>

    <ProposalReview
      v-else-if="proposalReviewing"
      :project-id="projectId"
      :deployment-id="deployment.id"
      :proposed-files="proposedFiles"
      :original-files="originalFiles"
      :execution-plan="plan"
      @apply="onApplyProposal"
      @discard="onDiscardProposal"
      @modify="onModifyProposal"
    />

    <template v-else>
      <div class="deploy-artifacts__body">
        <aside class="deploy-artifacts__sidebar" data-testid="deploy-artifacts-sidebar">
          <div class="deploy-artifacts__sidebar-header">
            <h3>Deployment artifacts</h3>
          </div>

          <div class="deploy-artifacts__pipeline">
            <div class="deploy-artifacts__phase-tabs" data-testid="deploy-artifacts-phase-tabs">
              <button
                type="button"
                class="deploy-artifacts__phase-tab"
                :class="{ 'deploy-artifacts__phase-tab--active': phase === 'deploy' }"
                data-testid="phase-tab-deploy"
                @click="phase = 'deploy'"
              >Deploy</button><button
                type="button"
                class="deploy-artifacts__phase-tab"
                :class="{ 'deploy-artifacts__phase-tab--active': phase === 'destroy' }"
                data-testid="phase-tab-destroy"
                @click="phase = 'destroy'"
              >Destroy</button>
            </div>
            <PipelineStepper
              :steps="currentPhaseSteps"
              :selected-step-id="selectedArtifactId"
              @select="onPipelineSelect"
            />
          </div>
        </aside>

        <div class="deploy-artifacts__editor">
          <template v-if="selectedBashPair">
            <ArtifactEditor
              v-show="scriptTab === 'deploy'"
              :project-id="projectId"
              :deployment-id="deployment.id"
              :artifact-id="selectedBashPair.deployId"
              :bash-pair="selectedBashPair"
              :script-tab="scriptTab"
              @script-tab-change="scriptTab = $event"
              @saved="onSaved"
            />
            <ArtifactEditor
              v-show="scriptTab === 'destroy'"
              :project-id="projectId"
              :deployment-id="deployment.id"
              :artifact-id="selectedBashPair.destroyId"
              :bash-pair="selectedBashPair"
              :script-tab="scriptTab"
              @script-tab-change="scriptTab = $event"
              @saved="onSaved"
            />
          </template>
          <ArtifactEditor
            v-else
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
    </template>

    <div v-if="proposeModalOpen" class="modal" @click.self="closeProposeModal">
      <div class="modal-content" data-testid="propose-change-modal">
        <button class="modal-close" type="button" @click="closeProposeModal">✕</button>
        <h3>Propose a change</h3>
        <p class="modal-lede">Describe the changes you'd like to make to the deployment artifacts. Any artifact can be modified or created.</p>
        <div class="form-group">
          <label for="propose-change-prompt">Change description</label>
          <textarea
            id="propose-change-prompt"
            v-model="proposePrompt"
            rows="8"
            data-testid="propose-change-prompt"
            placeholder="Describe what you'd like to change"
          />
        </div>
        <div v-if="proposeError" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ proposeError }}</p>
          </div>
        </div>
        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" @click="closeProposeModal">Cancel</button>
          <button
            class="p-button--positive is-dense"
            type="button"
            data-testid="submit-propose-change-btn"
            :disabled="!proposePrompt.trim() || proposalProposing"
            @click="submitProposal"
          >{{ proposalProposing ? 'Proposing…' : 'Propose' }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onUnmounted } from 'vue';
import ArtifactEditor from './ArtifactEditor.vue';
import AddArtifactModal from './AddArtifactModal.vue';
import PipelineStepper from './PipelineStepper.vue';
import DesignGenerationPanel from './DesignGenerationPanel.vue';
import ProposalReview from './ProposalReview.vue';
import {
  getExecutionPlan, setExecutionPlan, openProjectEvents,
  proposeProvisionChangeStream, applyProvisionChange,
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
const scriptTab             = ref('deploy');
const phase                  = ref('deploy');

const proposeModalOpen       = ref(false);
const proposePrompt          = ref('');
const proposeError           = ref(null);
const proposalProposing      = ref(false);
const proposalReviewing      = ref(false);
const proposedFiles          = ref({});
const originalFiles          = ref({});

let eventSource = null;

const currentPhaseSteps = computed(() =>
  phase.value === 'deploy' ? (plan.value.deploy_steps ?? []) : (plan.value.destroy_steps ?? [])
);

const proposalBody = computed(() => ({
  instructions: proposePrompt.value.trim(),
}));

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

function bashNameFromTitle(title) {
  if (!title) return null;
  const m = title.match(/^(?:deploy|destroy)-(.+\.sh)$/i);
  return m ? m[1] : null;
}

const bashPairs = computed(() => {
  const deployBash = (plan.value.deploy_steps ?? [])
    .filter(s => s.artifact?.kind === 'bash' && s.action === 'run');
  const destroyBash = (plan.value.destroy_steps ?? [])
    .filter(s => s.artifact?.kind === 'bash' && s.action === 'destroy');
  const pairs = [];
  const usedDestroy = new Set();
  for (const ds of deployBash) {
    const name = bashNameFromTitle(ds.artifact.title);
    const matchingDestroy = name
      ? destroyBash.find(s => {
          if (usedDestroy.has(s.artifact.id)) return false;
          return bashNameFromTitle(s.artifact.title) === name;
        })
      : undefined;
    if (matchingDestroy) usedDestroy.add(matchingDestroy.artifact.id);
    pairs.push({
      deployId:  ds.artifact.id,
      destroyId: matchingDestroy?.artifact.id ?? null,
      name:      name ?? ds.artifact.title,
    });
  }
  for (const s of destroyBash) {
    if (usedDestroy.has(s.artifact.id)) continue;
    const name = bashNameFromTitle(s.artifact.title);
    pairs.push({
      deployId:  null,
      destroyId: s.artifact.id,
      name:      name ?? s.artifact.title,
    });
  }
  return pairs;
});

function bashPairForArtifact(id) {
  if (!id) return null;
  return bashPairs.value.find(p => p.deployId === id || p.destroyId === id) ?? null;
}

const sidebarItems = computed(() => {
  const seen = new Set();
  const items = [];
  for (const step of (plan.value.deploy_steps ?? [])) {
    if (!step.artifact) continue;
    if (step.artifact.kind === 'bash' && step.action === 'run') {
      const pair = bashPairs.value.find(p => p.deployId === step.artifact.id);
      if (pair) {
        const key = pair.deployId ?? pair.destroyId;
        if (seen.has(key)) continue;
        seen.add(key);
        items.push({
          key,
          kind:  'bash',
          title: pair.name,
          pair,
        });
      }
    } else {
      if (seen.has(step.artifact.id)) continue;
      seen.add(step.artifact.id);
      items.push({
        key:   step.artifact.id,
        kind:  step.artifact.kind,
        title: step.artifact.title,
        pair:  null,
      });
    }
  }
  for (const step of (plan.value.destroy_steps ?? [])) {
    if (!step.artifact) continue;
    if (step.artifact.kind === 'bash' && step.action === 'destroy') {
      const pair = bashPairs.value.find(p => p.destroyId === step.artifact.id);
      if (pair && pair.deployId) continue;
      const key = pair ? (pair.deployId ?? pair.destroyId) : step.artifact.id;
      if (seen.has(key)) continue;
      seen.add(key);
      items.push({
        key,
        kind:  'bash',
        title: pair ? pair.name : step.artifact.title,
        pair,
      });
    } else {
      if (seen.has(step.artifact.id)) continue;
      seen.add(step.artifact.id);
      items.push({
        key:   step.artifact.id,
        kind:  step.artifact.kind,
        title: step.artifact.title,
        pair:  null,
      });
    }
  }
  return items;
});

const selectedBashPair = computed(() => {
  if (!selectedArtifactId.value) return null;
  return bashPairForArtifact(selectedArtifactId.value);
});

function isItemSelected(item) {
  if (item.pair) {
    return item.pair.deployId === selectedArtifactId.value || item.pair.destroyId === selectedArtifactId.value;
  }
  return item.key === selectedArtifactId.value;
}

function selectSidebarItem(item) {
  if (item.pair) {
    const id = item.pair.deployId ?? item.pair.destroyId;
    selectedArtifactId.value = id;
    scriptTab.value = (id === item.pair.deployId && item.pair.deployId) ? 'deploy' : 'destroy';
  } else {
    selectedArtifactId.value = item.key;
  }
}

function onPipelineSelect(artifactId) {
  selectedArtifactId.value = artifactId;
  const pair = bashPairForArtifact(artifactId);
  if (pair) {
    if (artifactId === pair.deployId) {
      scriptTab.value = 'deploy';
    } else if (artifactId === pair.destroyId) {
      scriptTab.value = 'destroy';
    }
  }
}

async function loadPlan() {
  try {
    plan.value = await getExecutionPlan(props.projectId, props.deployment.id);
  } catch {
    plan.value = { deploy_steps: [], destroy_steps: [] };
  }
}

function onSaved() {
}

async function onArtifactAdded(newArtifact) {
  addArtifactOpen.value = false;
  const isTerraform = newArtifact.kind === 'terraform' || newArtifact.kind === 'terragrunt';
  const action = isTerraform ? 'apply' : 'run';
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
  const destroySteps = plan.value.destroy_steps.map(s => ({
    artifact_id: s.artifact.id,
    action:      s.action,
    label:       s.label,
    depends_on:  (s.depends_on ?? []).map(depId => {
      const idx = plan.value.destroy_steps.findIndex(ds => ds.id === depId);
      return idx >= 0 ? idx : 0;
    }),
  }));
  if (isTerraform) {
    destroySteps.push({
      artifact_id: newArtifact.id,
      action: 'destroy',
      label: newArtifact.id,
      depends_on: [],
    });
  }
  try {
    await setExecutionPlan(props.projectId, props.deployment.id, {
      deploy_steps: deploySteps,
      destroy_steps: destroySteps,
    });
  } catch {}
  await loadPlan();
  const item = sidebarItems.value.find(i => i.key === newArtifact.id);
  if (item) {
    selectSidebarItem(item);
  } else {
    selectedArtifactId.value = newArtifact.id;
  }
}

function openProposeModal() {
  proposeModalOpen.value = true;
  proposePrompt.value = '';
  proposeError.value = null;
}

function closeProposeModal() {
  proposeModalOpen.value = false;
  proposePrompt.value = '';
  proposeError.value = null;
}

function submitProposal() {
  if (!proposePrompt.value.trim() || proposalProposing.value) return;
  proposalProposing.value = true;
  proposeError.value = null;
  proposeModalOpen.value = false;
}

function extractJsonBlock(text) {
  const match = text.match(/```json\s*\n([\s\S]*?)```/);
  if (match) {
    try {
      return JSON.parse(match[1]);
    } catch {}
  }
  try {
    return JSON.parse(text);
  } catch {}
  return null;
}

async function onProposalStreamDone(payload) {
  const answer = (payload?.answer || payload?.text || '').trim();
  if (!answer) {
    proposalProposing.value = false;
    proposeError.value = 'Failed to propose changes';
    return;
  }
  const proposed = extractJsonBlock(answer);
  if (!proposed) {
    proposalProposing.value = false;
    proposeError.value = 'Failed to parse proposed changes from AI response';
    return;
  }
  proposedFiles.value = proposed;
  originalFiles.value = {};
  proposalProposing.value = false;
  proposalReviewing.value = true;
}

function onProposalStreamCancel() {
  proposalProposing.value = false;
  proposeError.value = null;
}

async function onApplyProposal(filesToApply) {
  try {
    await applyProvisionChange(props.projectId, props.deployment.id, {
      files: filesToApply,
    });
    proposalReviewing.value = false;
    proposedFiles.value = {};
    originalFiles.value = {};
    emit('refresh');
    await loadPlan();
  } catch (e) {
    proposeError.value = e.message || 'Failed to apply changes';
  }
}

function onDiscardProposal() {
  proposalReviewing.value = false;
  proposedFiles.value = {};
  originalFiles.value = {};
}

function onModifyProposal() {
  proposalReviewing.value = false;
  proposedFiles.value = {};
  originalFiles.value = {};
  proposeModalOpen.value = true;
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

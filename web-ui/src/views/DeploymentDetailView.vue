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
        <button
          v-if="pendingProposalCount > 0"
          class="p-button--base is-dense deployment-review-btn"
          type="button"
          data-testid="review-toggle"
          @click="reviewOpen = !reviewOpen"
        >
          Review <span class="deployment-review-btn__count">{{ pendingProposalCount }}</span>
        </button>
      </div>

      <div
        v-if="isBroken"
        class="p-notification--caution deployment-broken-banner"
        data-testid="broken-issues-banner"
      >
        <div class="p-notification__content">
          <p class="p-notification__message">
            This deployment is broken —
            <router-link :to="`/issues?deployment=${deployment.id}`" data-testid="view-issues-link">View issues</router-link>
          </p>
        </div>
      </div>

      <nav class="deployment-tabs">
        <button
          v-for="card in cards"
          :key="card.id"
          type="button"
          class="deployment-tab"
          :class="{ 'deployment-tab--active': selectedCard === card.id }"
          :data-testid="`tab-${card.id}`"
          @click="selectedCard = card.id"
        >
          <span class="deployment-tab__label">{{ card.label }}</span>
          <span class="deployment-tab__status">{{ card.status }}</span>
          <span v-if="card.needsAttention" class="deployment-tab__dot" />
        </button>
      </nav>

      <div class="deployment-tab-content">
        <ContextArtifactsPanel
          v-if="selectedCard === 'context'"
          :project-id="projectId"
          :deployment="deployment"
          @refresh="load"
        />
        <DesignPanel
          v-else-if="selectedCard === 'design'"
          :project-id="projectId"
          :deployment="deployment"
          @refresh="load"
        />
        <ArtifactsPanel
          v-else-if="selectedCard === 'artifacts'"
          :project-id="projectId"
          :deployment="deployment"
          :runs="runs"
          :agents="agents"
          @refresh="load"
        />
        <GuidePanel
          v-else-if="selectedCard === 'guide'"
          :project-id="projectId"
          :deployment="deployment"
          @refresh="load"
        />
      </div>

      <Transition name="review-drawer">
        <div v-if="reviewOpen" class="review-drawer-overlay" @click.self="reviewOpen = false">
          <div class="review-drawer" data-testid="review-drawer">
            <div class="review-drawer__header">
              <span>Review proposals</span>
              <button class="review-drawer__close" type="button" @click="reviewOpen = false">✕</button>
            </div>
            <ReviewInbox
              :project-id="projectId"
              :deployment-id="deployment.id"
            />
          </div>
        </div>
      </Transition>
    </template>

    <div v-else class="deployment-detail-error">Failed to load deployment.</div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { useRoute } from 'vue-router';
import ContextArtifactsPanel from '../components/deployment/ContextArtifactsPanel.vue';
import DesignPanel from '../components/deployment/DesignPanel.vue';
import ArtifactsPanel from '../components/deployment/ArtifactsPanel.vue';
import GuidePanel from '../components/deployment/GuidePanel.vue';
import ReviewInbox from '../components/deployment/ReviewInbox.vue';
import { getProjectDeployment, listDeploymentRuns, listProjectAgents, listDeploymentProposals } from '../lib/api.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const route = useRoute();

const deployment    = ref(null);
const runs           = ref([]);
const agents         = ref([]);
const proposals      = ref([]);
const loading        = ref(false);
const selectedCard   = ref('context');
const reviewOpen      = ref(false);

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

const isBroken = computed(() => ['broken', 'destroy_failed'].includes(deployment.value?.infra_state));
const pendingProposalCount = computed(() => proposals.value.filter(p => p.status === 'pending').length);

const cards = computed(() => {
  const d = deployment.value;
  if (!d) return [];
  const contextCount = d.context_artifacts?.length ?? 0;
  return [
    {
      id: 'context',
      label: 'Context',
      status: contextCount ? `${contextCount}` : 'Empty',
      needsAttention: contextCount === 0,
    },
    {
      id: 'design',
      label: 'Design',
      status: d.design_doc ? 'Draft' : '—',
      needsAttention: !d.design_doc,
    },
    {
      id: 'artifacts',
      label: 'Artifacts',
      status: artifactStatus(d),
      needsAttention: false,
    },
    {
      id: 'guide',
      label: 'Guide',
      status: d.guide ? 'Draft' : '—',
      needsAttention: false,
    },
  ];
});

function artifactStatus(d) {
  if (!d.terraform_bundle) return '—';
  return `${d.infra_state ?? 'none'}`;
}

async function load() {
  if (!props.projectId || !deploymentId.value) return;
  loading.value = true;
  try {
    const [d, r, a, p] = await Promise.all([
      getProjectDeployment(props.projectId, deploymentId.value),
      listDeploymentRuns(props.projectId, deploymentId.value).catch(() => []),
      listProjectAgents(props.projectId).catch(() => []),
      listDeploymentProposals(props.projectId, deploymentId.value).catch(() => []),
    ]);
    deployment.value = d;
    runs.value        = r;
    agents.value       = a;
    proposals.value    = p;
  } catch {
    deployment.value = null;
  }
  loading.value = false;
}

watch(deploymentId, () => {
  selectedCard.value = 'context';
  reviewOpen.value = false;
  load();
}, { immediate: true });
</script>

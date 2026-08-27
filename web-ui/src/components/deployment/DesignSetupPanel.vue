<template>
  <div class="design-setup" data-testid="design-setup">
    <div class="design-setup__intro">
      <h2 class="p-heading--4">Generate a design</h2>
      <p class="u-text--muted">
        Pick a product template and select the artifacts that should inform the
        design, then generate the first version of the deployment design document.
      </p>
    </div>

    <div v-if="error" class="p-notification--negative">
      <div class="p-notification__content">
        <p class="p-notification__message">{{ error }}</p>
      </div>
    </div>

    <div class="design-setup__grid">
      <section class="design-setup__col">
        <div class="design-setup__step" data-testid="step-template">
          <span class="p-badge">1</span>
          <label for="design-template-select" class="design-setup__label">Product template</label>
        </div>
        <select
          id="design-template-select"
          v-model="selectedTemplateId"
          class="design-setup__select"
          data-testid="template-select"
          :disabled="busy"
        >
          <option value="">No template</option>
          <option v-for="t in templates" :key="t.id" :value="t.id">{{ t.name }}</option>
        </select>
        <p v-if="selectedTemplate" class="p-text--small u-text--muted">{{ selectedTemplate.description }}</p>
      </section>

      <section class="design-setup__col">
        <div class="design-setup__step design-setup__step--row" data-testid="step-artifacts">
          <span class="p-badge">2</span>
          <h3 class="p-heading--5">Context artifacts</h3>
          <span class="p-chip" data-testid="selection-count">{{ selectedCount }} selected</span>
          <span class="design-setup__bulk" v-if="artifacts.length">
            <button type="button" class="p-button--base is-dense" data-testid="select-all-artifacts" :disabled="busy" @click="selectAll">Select all</button>
            <button type="button" class="p-button--base is-dense" data-testid="clear-artifacts" :disabled="busy || !selectedArtifactIds.length" @click="clearAll">Clear</button>
          </span>
        </div>
        <router-link
          to="/artifacts"
          class="p-text--small"
          data-testid="artifacts-link"
        >Add or manage artifacts →</router-link>

        <div v-if="artifactsLoading" class="design-setup__loading">
          <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
        </div>
        <div v-else-if="!artifacts.length" class="design-setup__empty" data-testid="artifacts-empty">
          <p class="u-text--muted">This project has no artifacts yet.</p>
          <router-link to="/artifacts" class="p-text--small" data-testid="artifacts-link">Add an artifact →</router-link>
        </div>
        <ul v-else class="p-list--divided design-setup__list">
          <li v-for="a in artifacts" :key="a.id" class="p-list__item design-setup__item">
            <div class="p-checkbox design-setup__item-checkbox">
              <input
                type="checkbox"
                class="p-checkbox__input"
                :id="`artifact-checkbox-${a.id}`"
                :value="a.id"
                v-model="selectedArtifactIds"
                :data-testid="`artifact-checkbox-${a.id}`"
                :disabled="busy"
              />
              <label class="p-checkbox__label" :for="`artifact-checkbox-${a.id}`"></label>
            </div>
            <label class="design-setup__item-title" :for="`artifact-checkbox-${a.id}`">{{ a.title }}</label>
            <span class="artifact-kind-badge" :class="kindBadgeClass(a.kind)">{{ kindLabel(a.kind) }}</span>
            <span class="p-text--small u-text--muted design-setup__item-date">{{ formatDate(a.created_at) }}</span>
          </li>
        </ul>
      </section>
    </div>

    <div class="design-setup__footer">
      <p class="u-text--muted" data-testid="generation-summary">{{ generationSummary }}</p>
      <div class="design-setup__actions">
        <button
          class="p-button--positive"
          type="button"
          data-testid="generate-design-btn"
          :disabled="busy"
          @click="generate"
        >{{ busy ? 'Generating…' : 'Generate design' }}</button>
        <BusyStatus v-if="busy" text="Generating design…" />
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { listProjectArtifacts, listTemplates } from '../../lib/api.js';
import BusyStatus from './BusyStatus.vue';

const props = defineProps({
  projectId:    { type: String, required: true },
  deploymentId: { type: String, required: true },
  groupId:      { type: String, default: null },
});
const emit = defineEmits(['generate']);

const artifacts           = ref([]);
const templates           = ref([]);
const artifactsLoading    = ref(false);
const selectedTemplateId  = ref('');
const selectedArtifactIds = ref([]);
const busy                = ref(false);
const error               = ref(null);

const selectedTemplate = computed(() => templates.value.find(t => t.id === selectedTemplateId.value) ?? null);
const selectedCount    = computed(() => selectedArtifactIds.value.length);

const generationSummary = computed(() => {
  const parts = [];
  if (selectedTemplate.value) {
    parts.push(`the ${selectedTemplate.value.name} template`);
  } else {
    parts.push('no template');
  }
  const n = selectedCount.value;
  parts.push(n === 1 ? '1 artifact' : `${n} artifacts`);
  return `Generating with ${parts.join(' and ')}.`;
});

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

function formatDate(iso) {
  return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

function selectAll() {
  selectedArtifactIds.value = artifacts.value.map(a => a.id);
}

function clearAll() {
  selectedArtifactIds.value = [];
}

async function load() {
  artifactsLoading.value = true;
  error.value = null;
  try {
    const [listResult, tplsResult] = await Promise.allSettled([
      listProjectArtifacts(props.projectId),
      listTemplates(),
    ]);
    artifacts.value = listResult.status === 'fulfilled' ? listResult.value : [];
    templates.value = tplsResult.status === 'fulfilled' ? tplsResult.value : [];
    if (listResult.status === 'rejected' && tplsResult.status === 'rejected') {
      error.value = listResult.reason?.message || 'Failed to load context';
    } else if (listResult.status === 'rejected') {
      error.value = listResult.reason?.message || 'Failed to load artifacts';
    } else if (tplsResult.status === 'rejected') {
      error.value = tplsResult.reason?.message || 'Failed to load templates';
    }
  } finally {
    artifactsLoading.value = false;
  }
}

function generate() {
  busy.value = true;
  error.value = null;
  emit('generate', {
    artifact_ids: [...selectedArtifactIds.value],
    product_template_id: selectedTemplateId.value || null,
  });
  busy.value = false;
}

watch(() => [props.projectId, props.deploymentId], () => load(), { immediate: true });
</script>

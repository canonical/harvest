<template>
  <div class="design-setup" data-testid="design-setup">
    <div class="design-setup__intro">
      <h2 class="design-setup__title">Generate a design</h2>
      <p class="design-setup__lede">
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
          <span class="design-setup__step-num">1</span>
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
        <p v-if="selectedTemplate" class="design-setup__template-desc">{{ selectedTemplate.description }}</p>
      </section>

      <section class="design-setup__col">
        <div class="design-setup__step design-setup__step--row" data-testid="step-artifacts">
          <span class="design-setup__step-num">2</span>
          <h3 class="design-setup__subtitle">Context artifacts</h3>
          <span class="design-setup__count" data-testid="selection-count">{{ selectedCount }} selected</span>
          <span class="design-setup__bulk" v-if="artifacts.length">
            <button type="button" class="design-setup__bulk-btn" data-testid="select-all-artifacts" :disabled="busy" @click="selectAll">Select all</button>
            <button type="button" class="design-setup__bulk-btn" data-testid="clear-artifacts" :disabled="busy || !selectedArtifactIds.length" @click="clearAll">Clear</button>
          </span>
        </div>
        <router-link
          to="/artifacts"
          class="design-setup__artifacts-link"
          data-testid="artifacts-link"
        >Add or manage artifacts →</router-link>

        <div v-if="artifactsLoading" class="design-setup__loading">
          <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
        </div>
        <div v-else-if="!artifacts.length" class="design-setup__empty" data-testid="artifacts-empty">
          <p class="design-setup__empty-text">This project has no artifacts yet.</p>
          <router-link to="/artifacts" class="design-setup__empty-link" data-testid="artifacts-link">Add an artifact →</router-link>
        </div>
        <ul v-else class="design-setup__list">
          <li v-for="a in artifacts" :key="a.id" class="design-setup__item">
            <label class="design-setup__item-label">
              <input
                type="checkbox"
                :value="a.id"
                v-model="selectedArtifactIds"
                :data-testid="`artifact-checkbox-${a.id}`"
                :disabled="busy"
              />
              <span class="design-setup__item-title">{{ a.title }}</span>
              <span class="artifact-kind-badge" :class="kindBadgeClass(a.kind)">{{ kindLabel(a.kind) }}</span>
              <span class="design-setup__item-date">{{ formatDate(a.created_at) }}</span>
            </label>
          </li>
        </ul>
      </section>
    </div>

    <div class="design-setup__footer">
      <p class="design-setup__summary" data-testid="generation-summary">{{ generationSummary }}</p>
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
import { listProjectArtifacts, listGroupTemplates, generateDesign } from '../../lib/api.js';
import BusyStatus from './BusyStatus.vue';

const props = defineProps({
  projectId:    { type: String, required: true },
  deploymentId: { type: String, required: true },
  groupId:      { type: String, default: null },
});
const emit = defineEmits(['refresh']);

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
    const [list, tpls] = await Promise.all([
      listProjectArtifacts(props.projectId),
      props.groupId ? listGroupTemplates(props.groupId) : Promise.resolve([]),
    ]);
    artifacts.value = list;
    templates.value = tpls;
  } catch (e) {
    artifacts.value = [];
    templates.value = [];
    error.value = e.message || 'Failed to load context';
  } finally {
    artifactsLoading.value = false;
  }
}

async function generate() {
  busy.value = true;
  error.value = null;
  try {
    await generateDesign(props.projectId, props.deploymentId, {
      artifact_ids: [...selectedArtifactIds.value],
      product_template_id: selectedTemplateId.value || null,
    });
    emit('refresh');
  } catch (e) {
    error.value = e.message || 'Failed to generate design';
  } finally {
    busy.value = false;
  }
}

watch(() => [props.projectId, props.deploymentId, props.groupId], () => load(), { immediate: true });
</script>
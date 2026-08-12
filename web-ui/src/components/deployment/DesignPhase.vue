<template>
  <div class="design-phase">
    <div class="design-phase__left">
      <BusyStatus v-if="busyLabel" :text="busyLabel" />

      <button
        v-if="!deployment.design_doc"
        class="p-button--positive is-dense"
        data-testid="generate-design-btn"
        type="button"
        :disabled="busy"
        @click="generate"
      >{{ busy ? 'Generating…' : 'Generate design' }}</button>

      <template v-else>
        <button class="p-button--base is-dense" data-testid="get-decisions-btn" type="button" :disabled="busy" @click="loadDecisions">
          {{ busy ? 'Working…' : 'Get design decisions' }}
        </button>

        <div v-for="d in decisions" :key="d.id" class="form-group">
          <label :for="`design-d-${d.id}`">{{ d.text }}</label>
          <input :id="`design-d-${d.id}`" v-model="decisionAnswers[d.id]" type="text" :placeholder="d.suggested || ''" />
        </div>

        <div class="form-group">
          <label for="design-instructions">Custom instructions</label>
          <textarea
            id="design-instructions"
            v-model="instructions"
            rows="4"
            placeholder="Anything you'd like to change about the design"
          ></textarea>
        </div>

        <button class="p-button--positive is-dense" data-testid="revise-design-btn" type="button" :disabled="busy || !canRevise" @click="revise">
          {{ busy ? 'Revising…' : 'Revise design' }}
        </button>
      </template>

      <div v-if="error" class="p-notification--negative">
        <div class="p-notification__content">
          <p class="p-notification__message">{{ error }}</p>
        </div>
      </div>
    </div>

    <div class="design-phase__right" v-html="renderedDesign" />
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { renderMarkdown } from '../../lib/markdown.js';
import { getArtifact, generateDesign, generateDesignDecisions, reviseDesign } from '../../lib/api.js';
import BusyStatus from './BusyStatus.vue';

const props = defineProps({
  projectId:  { type: String, required: true },
  deployment: { type: Object, required: true },
});
const emit = defineEmits(['refresh']);

const designContent   = ref('');
const decisions        = ref([]);
const decisionAnswers  = ref({});
const instructions     = ref('');
const busy              = ref(false);
const activeAction       = ref(null);
const error             = ref(null);

const renderedDesign = computed(() => designContent.value ? renderMarkdown(designContent.value, {}, {}) : '');
const canRevise = computed(() => decisions.value.length > 0 || instructions.value.trim().length > 0);
const busyLabel = computed(() => ({
  generate:  'Generating design…',
  decisions: 'Getting design decisions…',
  revise:    'Revising design…',
}[activeAction.value] ?? null));

async function loadDesignContent() {
  if (!props.deployment.design_doc) {
    designContent.value = '';
    return;
  }
  try {
    const artifact = await getArtifact(props.deployment.design_doc.id);
    designContent.value = artifact.content || '';
  } catch {
    designContent.value = '';
  }
}

async function generate() {
  busy.value = true;
  activeAction.value = 'generate';
  error.value = null;
  try {
    await generateDesign(props.projectId, props.deployment.id);
    emit('refresh');
  } catch (e) {
    error.value = e.message || 'Failed to generate design';
  } finally {
    busy.value = false;
    activeAction.value = null;
  }
}

async function loadDecisions() {
  busy.value = true;
  activeAction.value = 'decisions';
  error.value = null;
  try {
    const result = await generateDesignDecisions(props.projectId, props.deployment.id);
    decisions.value = result.decisions ?? [];
    decisionAnswers.value = Object.fromEntries(decisions.value.map(d => [d.id, d.suggested || '']));
  } catch (e) {
    error.value = e.message || 'Failed to generate decisions';
  } finally {
    busy.value = false;
    activeAction.value = null;
  }
}

async function revise() {
  busy.value = true;
  activeAction.value = 'revise';
  error.value = null;
  try {
    const decisionsPayload = decisions.value
      .filter(d => (decisionAnswers.value[d.id] || '').trim())
      .map(d => ({ question: d.text, answer: decisionAnswers.value[d.id] }));
    await reviseDesign(props.projectId, props.deployment.id, {
      decisions: decisionsPayload,
      instructions: instructions.value.trim() || null,
    });
    decisions.value = [];
    instructions.value = '';
    await loadDesignContent();
    emit('refresh');
  } catch (e) {
    error.value = e.message || 'Failed to revise design';
  } finally {
    busy.value = false;
    activeAction.value = null;
  }
}

watch(() => props.deployment.design_doc?.id, loadDesignContent, { immediate: true });
</script>

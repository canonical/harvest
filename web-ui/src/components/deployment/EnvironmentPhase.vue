<template>
  <div class="environment-phase">
    <button
      v-if="!questions.length"
      class="p-button--positive is-dense"
      data-testid="generate-questions-btn"
      type="button"
      :disabled="generating"
      @click="generateQuestions"
    >{{ generating ? 'Generating…' : 'Generate questions' }}</button>

    <BusyStatus v-if="busyLabel" :text="busyLabel" />

    <div v-for="q in questions" :key="q.id" class="form-group">
      <label :for="`env-q-${q.id}`">{{ q.text }}</label>
      <input :id="`env-q-${q.id}`" v-model="answers[q.id]" type="text" />
    </div>

    <div class="form-group">
      <label for="environment-notes">Additional notes</label>
      <textarea
        id="environment-notes"
        v-model="notes"
        rows="4"
        placeholder="Anything else relevant to the customer's environment"
      ></textarea>
    </div>

    <div v-if="error" class="p-notification--negative">
      <div class="p-notification__content">
        <p class="p-notification__message">{{ error }}</p>
      </div>
    </div>

    <button class="p-button--positive is-dense" data-testid="save-environment-btn" type="button" :disabled="saving" @click="save">
      {{ saving ? 'Saving…' : 'Save' }}
    </button>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { generateEnvironmentQuestions, updateProjectDeployment } from '../../lib/api.js';
import BusyStatus from './BusyStatus.vue';

const props = defineProps({
  projectId:  { type: String, required: true },
  deployment: { type: Object, required: true },
});
const emit = defineEmits(['refresh']);

const questions   = ref([]);
const answers     = ref({});
const notes       = ref('');
const generating  = ref(false);
const saving      = ref(false);
const error       = ref(null);

const busyLabel = computed(() => {
  if (generating.value) return 'Generating environment questions…';
  if (saving.value)     return 'Saving environment description…';
  return null;
});

function resetFromDeployment() {
  questions.value = [];
  answers.value   = {};
  notes.value     = props.deployment.environment_description || '';
  error.value     = null;
}

async function generateQuestions() {
  generating.value = true;
  error.value = null;
  try {
    const result = await generateEnvironmentQuestions(props.projectId, props.deployment.id);
    questions.value = result.questions ?? [];
    answers.value = Object.fromEntries(questions.value.map(q => [q.id, '']));
  } catch (e) {
    error.value = e.message || 'Failed to generate questions';
  } finally {
    generating.value = false;
  }
}

async function save() {
  saving.value = true;
  error.value = null;
  try {
    const qaText = questions.value
      .map(q => `Q: ${q.text}\nA: ${answers.value[q.id] || '(not answered)'}`)
      .join('\n\n');
    const combined = [qaText, notes.value.trim()].filter(Boolean).join('\n\n');
    await updateProjectDeployment(props.projectId, props.deployment.id, { environment_description: combined });
    emit('refresh');
  } catch (e) {
    error.value = e.message || 'Failed to save';
  } finally {
    saving.value = false;
  }
}

watch(() => props.deployment.id, resetFromDeployment, { immediate: true });
</script>

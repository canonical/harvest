<template>
  <AppModal title="New preview" size="wide" @close="emit('close')">
    <form :id="formId" class="app-form" @submit.prevent="submit">
      <fieldset class="preview-kinds">
        <legend>What should people try?</legend>
        <label v-for="option in PREVIEW_KINDS" :key="option.id" class="preview-kind" :class="{ 'is-selected': kind === option.id }">
          <input v-model="kind" type="radio" name="preview-kind" :value="option.id" />
          <span class="preview-kind__label">{{ option.label }}</span>
          <span class="preview-kind__description">{{ option.description }}</span>
        </label>
      </fieldset>
      <template v-if="needsPort(kind)">
        <label :for="`${formId}-port`">Port the app listens on</label>
        <input :id="`${formId}-port`" v-model.number="port" type="number" min="1" max="65535" :aria-invalid="String(Boolean(problem))" :aria-describedby="`${formId}-port-help`" required />
        <p :id="`${formId}-port-help`" class="p-form-help-text" :class="{ 'is-error': problem }">{{ problem || 'Harvest sets PORT to this value and HOST to 0.0.0.0.' }}</p>
        <label :for="`${formId}-command`">Start command</label>
        <input :id="`${formId}-command`" v-model="startCommand" type="text" spellcheck="false" placeholder="npm run dev -- --host 0.0.0.0 --port $PORT" />
        <p class="p-form-help-text">Install dependencies and build first, in a chat or the setup script: this command should only start the app. Leave it empty when the app already runs.</p>
      </template>
      <p v-if="error" class="p-form-validation__message is-error" role="alert">{{ error }}</p>
    </form>
    <template #actions>
      <button class="p-button--base is-dense" type="button" @click="emit('close')">Cancel</button>
      <button class="p-button--positive is-dense" type="submit" :form="formId" :disabled="saving || Boolean(problem)">{{ saving ? 'Starting…' : 'Start preview' }}</button>
    </template>
  </AppModal>
</template>

<script setup>
import { computed, ref } from 'vue';
import AppModal from '../ui/AppModal.vue';
import { PREVIEW_KINDS, needsPort, portProblem, previewSettings } from '../../lib/preview-settings.js';

const props = defineProps({
  create: { type: Function, required: true },
  previews: { type: Array, default: () => [] },
});
const emit = defineEmits(['close', 'created']);

const formId = `new-preview-${Math.random().toString(36).slice(2, 8)}`;
const kind = ref('web');
const port = ref(3000);
const startCommand = ref('');
const saving = ref(false);
const error = ref('');
const problem = computed(() => portProblem(port.value, kind.value, props.previews));

async function submit() {
  if (problem.value) return;
  saving.value = true;
  error.value = '';
  try {
    const preview = await props.create(previewSettings({ kind: kind.value, port: port.value, startCommand: startCommand.value }));
    emit('created', preview);
  } catch (e) {
    error.value = e.message;
  } finally {
    saving.value = false;
  }
}
</script>

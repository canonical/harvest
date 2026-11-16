<template>
  <form class="preview-form" @submit.prevent="submit">
    <p class="u-text--muted">Ask the agent for a preview, or start one yourself. Prepare dependencies first: the start command should only start the app.</p>
    <label :for="`${uid}-kind`">Kind</label>
    <select :id="`${uid}-kind`" v-model="kind">
      <option value="web">Web app</option>
      <option value="api">HTTP API</option>
      <option value="cli">Command-line tool</option>
    </select>
    <template v-if="kind !== 'cli'">
      <label :for="`${uid}-port`">Port</label>
      <input :id="`${uid}-port`" v-model.number="port" type="number" min="1" max="65535" required />
      <label :for="`${uid}-command`">Start command</label>
      <input :id="`${uid}-command`" v-model="startCommand" type="text" placeholder="npm run dev -- --host 0.0.0.0 --port $PORT" />
    </template>
    <p v-if="error" class="code-error" role="alert">{{ error }}</p>
    <div class="preview-form__actions">
      <button class="p-button--positive is-dense" type="submit" :disabled="saving">{{ saving ? 'Starting…' : 'Start preview' }}</button>
      <button v-if="cancellable" class="p-button--base is-dense" type="button" @click="emit('cancel')">Cancel</button>
    </div>
  </form>
</template>

<script setup>
import { ref } from 'vue';

const props = defineProps({
  create: { type: Function, required: true },
  cancellable: { type: Boolean, default: false },
});
const emit = defineEmits(['created', 'cancel']);

const uid = `preview-form-${Math.random().toString(36).slice(2, 8)}`;
const kind = ref('web');
const port = ref(3000);
const startCommand = ref('');
const saving = ref(false);
const error = ref('');

async function submit() {
  saving.value = true;
  error.value = '';
  try {
    const needsPort = kind.value !== 'cli';
    const preview = await props.create({
      kind: kind.value,
      port: needsPort ? port.value : null,
      start_command: needsPort ? startCommand.value.trim() || null : null,
    });
    emit('created', preview);
  } catch (e) {
    error.value = e.message;
  } finally {
    saving.value = false;
  }
}
</script>

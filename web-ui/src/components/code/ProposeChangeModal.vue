<template>
  <AppModal :title="updating ? `Update ${requestName.toLowerCase()}` : `Open ${requestName.toLowerCase()}`" size="wide" @close="emit('close')">
    <p v-if="updating" class="modal-lede">Pushes the session's latest commits to the open {{ requestName.toLowerCase() }}.</p>
    <form v-else :id="formId" class="app-form" @submit.prevent="submit">
      <label :for="`${formId}-title`">Title</label>
      <input :id="`${formId}-title`" v-model="title" type="text" required />
      <label :for="`${formId}-description`">Description</label>
      <textarea :id="`${formId}-description`" v-model="description" rows="6" placeholder="What changed and why" />
      <p class="p-form-help-text">Commit everything first: Harvest pushes the session's commits from the server with your credentials.</p>
    </form>
    <p v-if="error" class="p-form-validation__message is-error" role="alert">{{ error }}</p>
    <template #actions>
      <button class="p-button--base is-dense" type="button" @click="emit('close')">Cancel</button>
      <button class="p-button--positive is-dense" type="button" :disabled="submitting || (!updating && !title.trim())" @click="submit">
        {{ submitting ? 'Pushing…' : updating ? 'Push commits' : `Open ${requestName.toLowerCase()}` }}
      </button>
    </template>
  </AppModal>
</template>

<script setup>
import { computed, ref } from 'vue';
import AppModal from '../ui/AppModal.vue';
import { changeRequestName, proposeChange } from '../../lib/code-api.js';

const props = defineProps({
  sessionId: { type: String, required: true },
  branch: { type: String, required: true },
  changeRequest: { type: Object, default: null },
  platform: { type: String, default: '' },
});
const emit = defineEmits(['close', 'proposed']);

const formId = `propose-${Math.random().toString(36).slice(2, 8)}`;
const updating = computed(() => props.changeRequest?.state === 'open');
const requestName = computed(() => changeRequestName(props.changeRequest?.platform ?? props.platform));
const title = ref(props.branch);
const description = ref('');
const submitting = ref(false);
const error = ref('');

async function submit() {
  submitting.value = true;
  error.value = '';
  try {
    emit('proposed', await proposeChange(props.sessionId, title.value.trim() || props.branch, description.value));
  } catch (e) {
    error.value = e.message;
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <form :id="formId" class="code-session-form" @submit.prevent="start">
    <label :for="`${formId}-branch`">Branch</label>
    <input :id="`${formId}-branch`" ref="branchInput" v-model="branch" type="text" :list="`${formId}-branches`" placeholder="harvest/my-change" :disabled="starting" />
    <datalist :id="`${formId}-branches`">
      <option v-for="name in branches" :key="name" :value="name" />
    </datalist>
    <label :for="`${formId}-base`">From</label>
    <select :id="`${formId}-base`" v-model="baseBranch" :disabled="starting">
      <option v-for="name in baseChoices" :key="name" :value="name">{{ name }}</option>
    </select>
    <button class="p-button--positive is-dense" type="submit" :disabled="!branch.trim() || starting">{{ starting ? 'Starting…' : 'Start' }}</button>
  </form>
  <p v-if="error" class="code-error" role="alert">{{ error }}</p>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import { useRouter } from 'vue-router';
import { openCodeSession } from '../../lib/code-api.js';

const props = defineProps({
  projectId: { type: String, required: true },
  branches: { type: Array, default: () => [] },
  defaultBranch: { type: String, default: '' },
  autofocus: { type: Boolean, default: false },
});

const router = useRouter();
const formId = `session-${Math.random().toString(36).slice(2, 8)}`;
const branchInput = ref(null);
const branch = ref('');
const baseBranch = ref(props.defaultBranch || props.branches[0] || '');
const starting = ref(false);
const error = ref('');

const baseChoices = computed(() => [...new Set([...props.branches, props.defaultBranch].filter(Boolean))]);

async function start() {
  starting.value = true;
  error.value = '';
  try {
    const session = await openCodeSession(props.projectId, branch.value.trim(), baseBranch.value);
    router.push(`/code/sessions/${session.id}`);
  } catch (e) {
    error.value = e.message;
  } finally {
    starting.value = false;
  }
}

onMounted(() => {
  if (props.autofocus) branchInput.value.focus();
});
</script>

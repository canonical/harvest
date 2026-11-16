<template>
  <div class="code-session">
    <header class="code-session__header">
      <div class="code-session__title">
        <router-link v-if="project" :to="`/code/projects/${project.id}`" class="code-back">{{ project.name }}</router-link>
        <h2>{{ session?.branch ?? '…' }}</h2>
        <span v-if="session" class="p-label" :class="`code-status--${session.status}`">{{ statusLabel }}</span>
        <a v-if="changeRequest" :href="changeRequest.url" target="_blank" rel="noopener" class="code-change-request">
          {{ changeRequestName(changeRequest.platform) }} · {{ changeRequest.state }}
        </a>
      </div>
      <div class="code-session__actions">
        <button v-if="session?.status === 'active'" class="p-button--positive is-dense" type="button" @click="proposing = !proposing">
          {{ changeRequest?.state === 'open' ? 'Update change request' : 'Propose change' }}
        </button>
        <button v-if="isOpen(session)" class="p-button--base is-dense" type="button" :disabled="closing" @click="close">End session</button>
      </div>
    </header>

    <form v-if="proposing" class="code-proposal" @submit.prevent="propose">
      <template v-if="changeRequest?.state !== 'open'">
        <label for="proposal-title">Title</label>
        <input id="proposal-title" v-model="proposalTitle" type="text" required />
        <label for="proposal-description">Description</label>
        <textarea id="proposal-description" v-model="proposalDescription" rows="4" />
      </template>
      <p v-else class="u-text--muted">Pushes the latest commits to the open change request.</p>
      <div class="code-proposal__actions">
        <button class="p-button--positive is-dense" type="submit" :disabled="submitting || (!proposalTitle.trim() && changeRequest?.state !== 'open')">
          {{ submitting ? 'Pushing…' : (changeRequest?.state === 'open' ? 'Push commits' : 'Open change request') }}
        </button>
        <button class="p-button--base is-dense" type="button" @click="proposing = false">Cancel</button>
      </div>
    </form>

    <p v-if="error" class="code-error" role="alert">{{ error }}</p>
    <pre v-if="session?.status === 'failed' && session.error" class="code-log">{{ session.error }}</pre>

    <SessionWorkspace v-if="details" :key="id" :session-id="id" :details="details" />
  </div>
</template>

<script setup>
import { ref, computed, watch, onUnmounted } from 'vue';
import SessionWorkspace from '../components/code/SessionWorkspace.vue';
import { getCodeSession, closeCodeSession, isOpen, proposeChange, changeRequestName } from '../lib/code-api.js';

const props = defineProps({ id: { type: String, required: true } });

const STATUS_LABELS = { starting: 'Starting container…', active: 'Active', closed: 'Ended', failed: 'Failed' };
const POLL_MS = 2000;

const session = ref(null);
const project = ref(null);
const details = ref(null);
const error = ref('');
const closing = ref(false);
const changeRequest = ref(null);
const proposing = ref(false);
const submitting = ref(false);
const proposalTitle = ref('');
const proposalDescription = ref('');
let timer = null;

const statusLabel = computed(() => STATUS_LABELS[session.value?.status] ?? session.value?.status);

function stopPolling() {
  clearTimeout(timer);
  timer = null;
}

async function load() {
  stopPolling();
  try {
    const data = await getCodeSession(props.id);
    session.value = data.session;
    project.value = data.project;
    changeRequest.value = data.change_request;
    details.value = { chats: data.chats, previews: data.previews };
    if (!proposalTitle.value) proposalTitle.value = data.session.branch;
    if (data.session.status === 'starting') timer = setTimeout(load, POLL_MS);
  } catch (e) {
    error.value = e.message;
  }
}

async function propose() {
  submitting.value = true;
  error.value = '';
  try {
    changeRequest.value = await proposeChange(props.id, proposalTitle.value.trim() || session.value.branch, proposalDescription.value);
    proposing.value = false;
  } catch (e) {
    error.value = e.message;
  } finally {
    submitting.value = false;
  }
}

async function close() {
  closing.value = true;
  try {
    await closeCodeSession(props.id);
    await load();
  } catch (e) {
    error.value = e.message;
  } finally {
    closing.value = false;
  }
}

watch(() => props.id, () => {
  details.value = null;
  load();
}, { immediate: true });
onUnmounted(stopPolling);
</script>

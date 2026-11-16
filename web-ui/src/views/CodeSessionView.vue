<template>
  <div class="code-session">
    <header class="code-session__header">
      <div class="code-session__title">
        <router-link v-if="project" :to="`/code/projects/${project.id}`" class="code-back">{{ project.name }}</router-link>
        <h1>{{ session?.branch ?? '…' }}</h1>
        <span v-if="session" class="session-status" :class="`session-status--${session.status}`" :title="statusLabel">
          <span class="session-status__dot" aria-hidden="true" />
          <span :class="{ 'u-off-screen': session.status === 'active' }">{{ statusLabel }}</span>
        </span>
        <a v-if="changeRequest" :href="changeRequest.url" target="_blank" rel="noopener" class="p-chip session-change-request">
          <span class="p-chip__lead">{{ changeRequestName(changeRequest.platform) }}</span>
          <span class="p-chip__value">{{ changeRequest.state }} ↗</span>
        </a>
      </div>
      <div class="code-session__actions">
        <button v-if="session?.status === 'active'" class="p-button--positive is-dense" type="button" @click="proposing = true">
          {{ changeRequest?.state === 'open' ? 'Push to change request' : 'Propose change' }}
        </button>
        <MenuButton v-if="isOpen(session)" label="Session actions" :items="SESSION_ACTIONS" button-class="p-button--base is-dense has-icon" @select="onAction">
          <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="19" cy="12" r="2"/></svg>
        </MenuButton>
      </div>
    </header>

    <div v-if="error" class="p-notification--negative session-notice">
      <div class="p-notification__content">
        <p class="p-notification__message">{{ error }}</p>
        <button class="p-notification__close" type="button" aria-label="Dismiss" @click="error = ''">Close</button>
      </div>
    </div>
    <div v-if="session?.status === 'failed' && session.error" class="p-notification--negative session-notice">
      <div class="p-notification__content">
        <h5 class="p-notification__title">The session could not start</h5>
        <p class="p-notification__message">Open a new session to try again.</p>
      </div>
      <div class="p-notification__actions">
        <button class="p-button--base is-dense u-no-margin--bottom" type="button" @click="showLog = !showLog">{{ showLog ? 'Hide log' : 'Show log' }}</button>
      </div>
      <pre v-if="showLog" class="code-log">{{ session.error }}</pre>
    </div>

    <SessionWorkspace v-if="details" :key="id" :session-id="id" :details="details" />

    <ProposeChangeModal
      v-if="proposing && session"
      :session-id="id"
      :branch="session.branch"
      :change-request="changeRequest"
      @close="proposing = false"
      @proposed="onProposed"
    />
  </div>
</template>

<script setup>
import { ref, computed, watch, onUnmounted } from 'vue';
import SessionWorkspace from '../components/code/SessionWorkspace.vue';
import ProposeChangeModal from '../components/code/ProposeChangeModal.vue';
import MenuButton from '../components/ui/MenuButton.vue';
import { confirm } from '../composables/useConfirm.js';
import { showToast } from '../composables/useToasts.js';
import { getCodeSession, closeCodeSession, isOpen, changeRequestName } from '../lib/code-api.js';

const props = defineProps({ id: { type: String, required: true } });

const STATUS_LABELS = { starting: 'Starting container…', active: 'Active', closed: 'Ended', failed: 'Failed' };
const SESSION_ACTIONS = [{ id: 'end', label: 'End session…', danger: true }];
const POLL_MS = 2000;

const session = ref(null);
const project = ref(null);
const details = ref(null);
const error = ref('');
const changeRequest = ref(null);
const proposing = ref(false);
const showLog = ref(false);
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
    if (data.session.status === 'starting') timer = setTimeout(load, POLL_MS);
  } catch (e) {
    error.value = e.message;
  }
}

function onProposed(request) {
  changeRequest.value = request;
  proposing.value = false;
  showToast({ message: `${changeRequestName(request.platform)} is ${request.state}` });
}

async function onAction(item) {
  if (item.id !== 'end') return;
  const confirmed = await confirm({
    title: 'End this session?',
    message: 'Its container is deleted, every preview stops and shared preview links stop working. Pushed commits and change requests stay.',
    confirmLabel: 'End session',
    danger: true,
  });
  if (!confirmed) return;
  try {
    await closeCodeSession(props.id);
    await load();
    showToast({ message: 'Session ended' });
  } catch (e) {
    error.value = e.message;
  }
}

watch(() => props.id, () => {
  details.value = null;
  load();
}, { immediate: true });
onUnmounted(stopPolling);
</script>

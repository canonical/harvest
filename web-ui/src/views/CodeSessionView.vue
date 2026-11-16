<template>
  <div class="code-session">
    <header class="code-session__header">
      <div class="code-session__title">
        <router-link v-if="project" :to="`/code/projects/${project.id}`" class="code-back">{{ project.name }}</router-link>
        <h2>{{ session?.branch ?? '…' }}</h2>
        <span v-if="session" class="p-label" :class="`code-status--${session.status}`">{{ statusLabel }}</span>
      </div>
      <div class="code-session__actions">
        <div class="code-tabs" role="tablist">
          <button v-for="tab in TABS" :key="tab.id" type="button" role="tab" :aria-selected="view === tab.id" :class="{ 'is-active': view === tab.id }" @click="view = tab.id">{{ tab.label }}</button>
        </div>
        <button v-if="isOpen(session)" class="p-button--base is-dense" type="button" :disabled="closing" @click="close">End session</button>
      </div>
    </header>

    <p v-if="error" class="code-error" role="alert">{{ error }}</p>
    <pre v-if="session?.status === 'failed' && session.error" class="code-log">{{ session.error }}</pre>

    <div v-show="view === 'chat'" class="code-session__chat">
      <ChatPaneContent
        v-if="session?.conversation_id"
        :key="session.id"
        :tab-id="`code-${session.id}`"
        :conversation-id="session.conversation_id"
        :code-session-id="session.id"
      />
    </div>

    <div v-if="view === 'changes'" class="code-session__changes">
      <p v-if="changesError" class="u-text--muted">{{ changesError }}</p>
      <template v-else-if="changes">
        <ul v-if="changes.commits.length" class="code-commits">
          <li v-for="commit in changes.commits" :key="commit"><code>{{ commit }}</code></li>
        </ul>
        <p v-if="!changes.diff.trim()" class="u-text--muted">No changes yet.</p>
        <pre v-else class="code-diff"><span v-for="(line, i) in diffLines" :key="i" :class="line.kind">{{ line.text }}
</span></pre>
      </template>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onUnmounted } from 'vue';
import ChatPaneContent from '../components/chat/ChatPaneContent.vue';
import { getCodeSession, closeCodeSession, getSessionChanges, isOpen } from '../lib/code-api.js';
import { classifyDiffLines } from '../lib/diff-lines.js';

const props = defineProps({ id: { type: String, required: true } });

const TABS = [
  { id: 'chat', label: 'Chat' },
  { id: 'changes', label: 'Changes' },
];
const STATUS_LABELS = { starting: 'Starting container…', active: 'Active', closed: 'Ended', failed: 'Failed' };
const POLL_MS = 2000;

const session = ref(null);
const project = ref(null);
const view = ref('chat');
const changes = ref(null);
const changesError = ref('');
const error = ref('');
const closing = ref(false);
let timer = null;

const statusLabel = computed(() => STATUS_LABELS[session.value?.status] ?? session.value?.status);
const diffLines = computed(() => classifyDiffLines(changes.value?.diff ?? ''));

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
    if (data.session.status === 'starting') timer = setTimeout(load, POLL_MS);
  } catch (e) {
    error.value = e.message;
  }
}

async function loadChanges() {
  changesError.value = '';
  try {
    changes.value = await getSessionChanges(props.id);
  } catch (e) {
    changesError.value = e.message;
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

watch(() => props.id, load, { immediate: true });
watch(view, (tab) => { if (tab === 'changes') loadChanges(); });
onUnmounted(stopPolling);
</script>

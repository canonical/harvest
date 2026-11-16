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
        <div class="code-tabs" role="tablist">
          <button v-for="tab in TABS" :key="tab.id" type="button" role="tab" :aria-selected="view === tab.id" :class="{ 'is-active': view === tab.id }" @click="view = tab.id">{{ tab.label }}</button>
        </div>
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

    <div v-show="view === 'chat'" class="code-session__chat">
      <ChatPaneContent
        v-if="session?.conversation_id"
        :key="session.id"
        :tab-id="`code-${session.id}`"
        :conversation-id="session.conversation_id"
        :code-session-id="session.id"
      />
    </div>

    <div v-if="view === 'preview'" class="code-session__preview">
      <p v-if="!preview" class="u-text--muted">No preview yet. Ask the agent to set one up, for example “let me try it”.</p>
      <template v-else>
        <div class="preview-share">
          <label for="preview-audience">Shared with</label>
          <select id="preview-audience" :value="preview.audience" :disabled="sharing" @change="share($event.target.value)">
            <option v-for="audience in AUDIENCES" :key="audience.id" :value="audience.id">{{ audience.label }}</option>
          </select>
          <template v-if="preview.audience !== 'owner'">
            <input class="preview-share__link" :value="link" readonly aria-label="Share link" @focus="$event.target.select()" />
            <button class="p-button--base is-dense" type="button" @click="copyLink">{{ copied ? 'Copied' : 'Copy link' }}</button>
          </template>
        </div>
        <PreviewPanel :preview-id="preview.id" />
      </template>
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
import { getCodeSession, closeCodeSession, getSessionChanges, isOpen, proposeChange, changeRequestName } from '../lib/code-api.js';
import { classifyDiffLines } from '../lib/diff-lines.js';
import PreviewPanel from '../components/code/PreviewPanel.vue';
import { AUDIENCES, getSessionPreview, sharePreview, shareUrl } from '../lib/preview-api.js';

const props = defineProps({ id: { type: String, required: true } });

const TABS = [
  { id: 'chat', label: 'Chat' },
  { id: 'changes', label: 'Changes' },
  { id: 'preview', label: 'Preview' },
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
const changeRequest = ref(null);
const proposing = ref(false);
const submitting = ref(false);
const proposalTitle = ref('');
const proposalDescription = ref('');
const preview = ref(null);
const sharing = ref(false);
const copied = ref(false);
const link = computed(() => (preview.value ? shareUrl(preview.value.id) : ''));
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
    changeRequest.value = data.change_request;
    if (!proposalTitle.value) proposalTitle.value = data.session.branch;
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

watch(() => props.id, load, { immediate: true });
async function loadPreview() {
  try {
    preview.value = (await getSessionPreview(props.id)).preview;
  } catch (e) {
    error.value = e.message;
  }
}

async function share(audience) {
  sharing.value = true;
  try {
    await sharePreview(props.id, audience);
    preview.value = { ...preview.value, audience };
  } catch (e) {
    error.value = e.message;
  } finally {
    sharing.value = false;
  }
}

async function copyLink() {
  try {
    await navigator.clipboard.writeText(link.value);
    copied.value = true;
    setTimeout(() => { copied.value = false; }, 1500);
  } catch {
    copied.value = false;
  }
}

watch(view, (tab) => {
  if (tab === 'changes') loadChanges();
  if (tab === 'preview') loadPreview();
});
onUnmounted(stopPolling);
</script>

<template>
  <div class="agents-view">
    <div class="page-header">
      <h2>Agents</h2>
      <button id="install-agent-btn" class="p-button--positive" type="button" @click="openInstallModal">Add agent</button>
    </div>

    <template v-if="agents.length > 0">
      <table class="p-table">
        <thead>
          <tr>
            <th>Status</th>
            <th>Hostname</th>
            <th>Last seen</th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="agent in agents" :key="agent.id">
            <td>
              <span
                class="p-label agent-status"
                :class="agent.online ? 'p-label--positive agent-status--online' : 'agent-status--offline'"
              >
                {{ agent.online ? 'Online' : 'Offline' }}
              </span>
            </td>
            <td>{{ agent.hostname || agent.id }}</td>
            <td>{{ relativeTime(agent.last_seen) }}</td>
            <td>
              <button class="p-button--negative p-button--small" type="button" @click="handleDelete(agent)">Delete</button>
            </td>
          </tr>
        </tbody>
      </table>
    </template>

    <p v-else class="agents-empty">No agents registered for this project.</p>

    <div v-if="deletingAgent" class="modal" @click.self="deletingAgent = null">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="deletingAgent = null">✕</button>
        <h3>Delete agent</h3>
        <p>
          Delete agent <strong>{{ deletingAgent.hostname || deletingAgent.id }}</strong>?
          <template v-if="deletingAgent.online"> This agent is currently <strong>online</strong>.</template>
          This cannot be undone.
        </p>
        <div class="modal-actions">
          <button class="p-button--base" type="button" @click="deletingAgent = null">Cancel</button>
          <button class="p-button--negative" type="button" :disabled="deleting" @click="confirmDelete">Delete</button>
        </div>
      </div>
    </div>

    <div v-if="showModal" id="install-modal" class="modal" @click.self="closeInstallModal">
      <div class="modal-content modal-content--wide">
        <button id="install-modal-close" class="modal-close" type="button" @click="closeInstallModal">✕</button>
        <h3>Install agent</h3>
        <p class="install-note">Run this command on the machine you want to add as an agent.</p>
        <div class="install-cmd-wrap">
          <code class="install-cmd-code">{{ installCmd }}</code>
          <button id="copy-install-cmd" class="install-cmd-copy" :class="{ 'is-copied': copied }" type="button" @click="copyCmd">{{ copied ? 'Copied!' : 'Copy' }}</button>
        </div>
        <p class="install-note install-note--muted">Port 443 on <strong>{{ serverUrl }}</strong> must be reachable from the agent machine for the connection to work.</p>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue';
import { listProjectAgents, rotateInstallToken, deleteAgent } from '../lib/api.js';

const props = defineProps({ projectId: { type: String, required: true } });

const agents        = ref([]);
const showModal     = ref(false);
const installCmd    = ref('');
const serverUrl     = ref('');
const copied        = ref(false);
const deletingAgent = ref(null);
const deleting      = ref(false);
let refreshTimer = null;

async function load() {
  try {
    agents.value = await listProjectAgents(props.projectId);
  } catch {}
}

async function openInstallModal() {
  serverUrl.value  = window.location.origin;
  installCmd.value = `curl -fsSL ${serverUrl.value}/agents/${props.projectId}/install.sh | sudo bash`;
  showModal.value = true;
  try {
    await rotateInstallToken(props.projectId);
  } catch {}
}

function closeInstallModal() {
  showModal.value = false;
}

async function copyCmd() {
  await navigator.clipboard.writeText(installCmd.value).catch(() => {});
  copied.value = true;
  setTimeout(() => { copied.value = false; }, 1800);
}

function handleDelete(agent) {
  deletingAgent.value = agent;
}

async function confirmDelete() {
  const agent = deletingAgent.value;
  if (!agent) return;
  deleting.value = true;
  try {
    await deleteAgent(props.projectId, agent.id);
    await load();
    deletingAgent.value = null;
  } catch {
  } finally {
    deleting.value = false;
  }
}

function relativeTime(iso) {
  if (!iso) return '—';
  const s = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (s < 60)    return 'just now';
  if (s < 3600)  return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

onMounted(() => {
  load();
  refreshTimer = setInterval(load, 15_000);
});

onUnmounted(() => {
  clearInterval(refreshTimer);
});
</script>

<template>
  <div class="agents-view">
    <div class="page-header">
      <h2>Agents</h2>
      <button id="install-agent-btn" class="p-button--positive" type="button" @click="handleAddAgentClick">Add agent</button>
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
            <td>
              {{ agent.hostname || agent.id }}
              <span v-if="agent.provider === 'lxd'" class="agent-provider-badge">LXD</span>
            </td>
            <td>{{ relativeTime(agent.last_seen) }}</td>
            <td>
              <div class="agent-row-actions">
                <router-link
                  :to="`/agents/${agent.id}/console`"
                  class="console-icon-btn"
                  title="Open console"
                  aria-label="Open console"
                >
                  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>
                </router-link>
                <button
                  class="console-icon-btn console-icon-btn--danger"
                  type="button"
                  title="Delete agent"
                  aria-label="Delete agent"
                  @click="handleDelete(agent)"
                >
                  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/></svg>
                </button>
              </div>
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
          <template v-if="deletingAgent.provider === 'lxd'"> The LXD container backing this agent will also be deleted.</template>
          This cannot be undone.
        </p>
        <div class="modal-actions">
          <button class="p-button--base" type="button" @click="deletingAgent = null">Cancel</button>
          <button class="p-button--negative" type="button" :disabled="deleting" @click="confirmDelete">Delete</button>
        </div>
      </div>
    </div>

    <div v-if="showChoiceModal" id="agent-choice-modal" class="modal" @click.self="showChoiceModal = false">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="showChoiceModal = false">✕</button>
        <h3>Add agent</h3>
        <div class="agent-choice-options">
          <button id="choice-manual-btn" type="button" class="agent-choice-option" @click="chooseManualInstall">
            <svg class="agent-choice-option__icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="2" width="20" height="8" rx="2" ry="2"/><rect x="2" y="14" width="20" height="8" rx="2" ry="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/></svg>
            <span class="agent-choice-option__text">
              <span class="agent-choice-option__title">Install agent on existing machine</span>
              <span class="agent-choice-option__desc">Run an install command on a machine you already manage.</span>
            </span>
          </button>
          <button id="choice-lxd-btn" type="button" class="agent-choice-option" @click="chooseManagedAgent">
            <svg class="agent-choice-option__icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"/></svg>
            <span class="agent-choice-option__text">
              <span class="agent-choice-option__title">Let Harvest create and manage agent</span>
              <span class="agent-choice-option__desc">Harvest provisions and manages an LXD container for you.</span>
            </span>
          </button>
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

    <div v-if="showManagedModal" id="managed-agent-modal" class="modal" @click.self="closeManagedModal">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="closeManagedModal">✕</button>
        <h3>Let Harvest create and manage agent</h3>

        <template v-if="provisionSteps.length === 0">
          <div class="form-field">
            <label for="managed-agent-name">Name</label>
            <input id="managed-agent-name" type="text" v-model="managedName" placeholder="e.g. build-runner" />
          </div>

          <div class="form-field">
            <label for="managed-agent-description">Description</label>
            <textarea id="managed-agent-description" v-model="managedDescription" rows="2" placeholder="Optional"></textarea>
          </div>

          <div class="form-field">
            <label>Size</label>
            <div class="flavor-select">
              <button
                id="flavor-select-toggle"
                ref="flavorToggleRef"
                type="button"
                class="flavor-select__toggle"
                :aria-expanded="flavorDropdownOpen"
                :disabled="flavorsLoading || !flavors.length"
                @click="toggleFlavorDropdown"
              >
                <span>{{ selectedFlavor ? selectedFlavor.label : (flavorsLoading ? 'Loading…' : 'No sizes available') }}</span>
                <span v-if="selectedFlavor" class="flavor-select__badge">{{ selectedFlavor.cpu }} vCPU · {{ formatMemory(selectedFlavor.memory_mib) }}</span>
              </button>
              <Teleport to="body">
                <div
                  v-if="flavorDropdownOpen"
                  class="flavor-select__dropdown flavor-select__dropdown--teleported"
                  :style="flavorDropdownStyle"
                >
                  <button
                    v-for="f in flavors"
                    :key="f.id"
                    :id="`flavor-option-${f.id}`"
                    type="button"
                    class="flavor-select__item"
                    @click="selectFlavor(f)"
                  >
                    <span class="flavor-select__name">{{ f.label }}</span>
                    <span class="flavor-select__badge">{{ f.cpu }} vCPU · {{ formatMemory(f.memory_mib) }}</span>
                  </button>
                </div>
              </Teleport>
            </div>
          </div>

          <p v-if="managedError" class="managed-agent-error">{{ managedError }}</p>

          <div class="modal-actions">
            <button class="p-button--base" type="button" @click="closeManagedModal">Cancel</button>
            <button
              id="create-managed-agent-btn"
              class="p-button--positive"
              type="button"
              :disabled="!managedName.trim() || !selectedFlavor"
              @click="submitManagedAgent"
            >
              Create agent
            </button>
          </div>
        </template>

        <template v-else>
          <ProvisionSteps id="provision-steps" :steps="provisionSteps" />

          <p v-if="managedError" class="managed-agent-error">{{ managedError }}</p>

          <div class="modal-actions">
            <button v-if="managedError" class="p-button--base" type="button" @click="resetManagedForm">Try again</button>
            <button class="p-button--base" type="button" @click="closeManagedModal">{{ provisionDone || managedError ? 'Close' : 'Cancel' }}</button>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted, nextTick } from 'vue';
import {
  listProjectAgents, rotateInstallToken, deleteAgent,
  getAgentFlavors, provisionLxdAgent,
} from '../lib/api.js';
import {
  initialProvisionSteps, applyProvisionEvent, isProvisionDone, isProvisionError,
} from '../lib/lxd-provision.js';
import { useAuthStore } from '../stores/auth.js';
import ProvisionSteps from '../components/agents/ProvisionSteps.vue';

const PROVISION_WAIT_ATTEMPTS     = 15;
const PROVISION_WAIT_INTERVAL_MS  = 1000;

const props = defineProps({ projectId: { type: String, required: true } });

const auth = useAuthStore();

const agents        = ref([]);
const showModal     = ref(false);
const installCmd    = ref('');
const serverUrl     = ref('');
const copied        = ref(false);
const deletingAgent = ref(null);
const deleting      = ref(false);
let refreshTimer = null;

const showChoiceModal  = ref(false);
const showManagedModal = ref(false);
const flavors          = ref([]);
const flavorsLoading   = ref(false);
const flavorDropdownOpen = ref(false);
const flavorToggleRef    = ref(null);
const flavorDropdownStyle = ref({});
const selectedFlavor   = ref(null);
const managedName        = ref('');
const managedDescription = ref('');
const managedError        = ref('');
const provisionSteps      = ref([]);
const provisionDone       = ref(false);
let provisionWaitToken    = 0;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function load() {
  try {
    agents.value = await listProjectAgents(props.projectId);
  } catch {}
}

async function handleAddAgentClick() {
  if (auth.features.lxd) {
    showChoiceModal.value = true;
  } else {
    await openInstallModal();
  }
}

async function chooseManualInstall() {
  showChoiceModal.value = false;
  await openInstallModal();
}

async function chooseManagedAgent() {
  provisionWaitToken += 1;
  showChoiceModal.value = false;
  managedName.value = '';
  managedDescription.value = '';
  managedError.value = '';
  provisionSteps.value = [];
  provisionDone.value = false;
  showManagedModal.value = true;
  flavorsLoading.value = true;
  try {
    flavors.value = await getAgentFlavors(props.projectId);
    selectedFlavor.value = flavors.value.find(f => f.id === 'small') || flavors.value[0] || null;
  } catch {
    flavors.value = [];
    selectedFlavor.value = null;
  } finally {
    flavorsLoading.value = false;
  }
}

function closeManagedModal() {
  provisionWaitToken += 1;
  showManagedModal.value = false;
  flavorDropdownOpen.value = false;
}

function resetManagedForm() {
  provisionSteps.value = [];
  provisionDone.value = false;
  managedError.value = '';
}

function updateFlavorDropdownPosition() {
  const el = flavorToggleRef.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  flavorDropdownStyle.value = {
    position: 'fixed',
    top:  `${rect.bottom + 4}px`,
    left: `${rect.left}px`,
    width: `${rect.width}px`,
  };
}

function closeFlavorDropdown() {
  flavorDropdownOpen.value = false;
}

function handleOutsideScroll(event) {
  if (event.target?.closest?.('.flavor-select__dropdown')) return;
  closeFlavorDropdown();
}

async function toggleFlavorDropdown() {
  if (flavorDropdownOpen.value) {
    flavorDropdownOpen.value = false;
    return;
  }
  flavorDropdownOpen.value = true;
  await nextTick();
  updateFlavorDropdownPosition();
}

function selectFlavor(f) {
  selectedFlavor.value = f;
  flavorDropdownOpen.value = false;
}

function formatMemory(mib) {
  return mib >= 1024 ? `${mib / 1024} GiB` : `${mib} MiB`;
}

async function submitManagedAgent() {
  if (!managedName.value.trim() || !selectedFlavor.value) return;
  managedError.value = '';
  provisionDone.value = false;
  provisionSteps.value = initialProvisionSteps();

  try {
    await provisionLxdAgent(props.projectId, {
      name:        managedName.value.trim(),
      description: managedDescription.value.trim(),
      flavor:      selectedFlavor.value.id,
    }, (event) => {
      provisionSteps.value = applyProvisionEvent(provisionSteps.value, event);
      if (isProvisionDone(event)) {
        provisionDone.value = true;
        waitForAgentThenClose(event.hostname);
      } else if (isProvisionError(event)) {
        managedError.value = event.message;
      }
    });
  } catch (e) {
    managedError.value = e.message || 'Failed to create agent';
  }
}

async function waitForAgentThenClose(hostname) {
  const token = ++provisionWaitToken;
  for (let attempt = 0; attempt < PROVISION_WAIT_ATTEMPTS; attempt += 1) {
    await load();
    if (token !== provisionWaitToken) return;
    if (agents.value.some(a => a.hostname === hostname)) break;
    await sleep(PROVISION_WAIT_INTERVAL_MS);
    if (token !== provisionWaitToken) return;
  }
  if (token === provisionWaitToken) closeManagedModal();
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
  window.addEventListener('resize', closeFlavorDropdown);
  window.addEventListener('scroll', handleOutsideScroll, true);
});

onUnmounted(() => {
  provisionWaitToken += 1;
  clearInterval(refreshTimer);
  window.removeEventListener('resize', closeFlavorDropdown);
  window.removeEventListener('scroll', handleOutsideScroll, true);
});
</script>

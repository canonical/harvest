<template>
  <button id="install-agent-btn" class="p-button--positive is-dense" type="button" @click="handleAddAgentClick">Add agent</button>

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
          <button class="p-button--base is-dense" type="button" @click="closeManagedModal">Cancel</button>
          <button
            id="create-managed-agent-btn"
            class="p-button--positive is-dense"
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
          <button v-if="managedError" class="p-button--base is-dense" type="button" @click="resetManagedForm">Try again</button>
          <button class="p-button--base is-dense" type="button" @click="closeManagedModal">{{ provisionDone || managedError ? 'Close' : 'Cancel' }}</button>
        </div>
      </template>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted, nextTick } from 'vue';
import {
  rotateInstallToken, getAgentFlavors, provisionLxdAgent,
} from '../../lib/api.js';
import {
  initialProvisionSteps, applyProvisionEvent, isProvisionDone, isProvisionError,
} from '../../lib/lxd-provision.js';
import { useAuthStore } from '../../stores/auth.js';
import ProvisionSteps from './ProvisionSteps.vue';

const PROVISION_WAIT_ATTEMPTS    = 15;
const PROVISION_WAIT_INTERVAL_MS = 1000;

const props = defineProps({
  projectId: { type: String, required: true },
  agents:    { type: Array, default: () => [] },
  reload:    { type: Function, default: async () => {} },
});
const emit = defineEmits(['added']);

const auth = useAuthStore();

const showModal     = ref(false);
const installCmd    = ref('');
const serverUrl     = ref('');
const copied        = ref(false);
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
    await props.reload();
    if (token !== provisionWaitToken) return;
    if (props.agents.some(a => a.hostname === hostname)) break;
    await sleep(PROVISION_WAIT_INTERVAL_MS);
    if (token !== provisionWaitToken) return;
  }
  if (token === provisionWaitToken) {
    closeManagedModal();
    emit('added');
  }
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

onMounted(() => {
  window.addEventListener('resize', closeFlavorDropdown);
  window.addEventListener('scroll', handleOutsideScroll, true);
});

onUnmounted(() => {
  provisionWaitToken += 1;
  window.removeEventListener('resize', closeFlavorDropdown);
  window.removeEventListener('scroll', handleOutsideScroll, true);
});

defineExpose({ closeManagedModal });
</script>

<template>
  <div class="deploy-setup" data-testid="deploy-setup">
    <div class="deploy-setup__intro">
      <p class="deploy-setup__eyebrow">Step 1 of 2</p>
      <h2 class="deploy-setup__title">Connect agents</h2>
      <p class="deploy-setup__lede">
        Connect at least one Harvest agent to run the deployment.
      </p>
    </div>

    <section class="deploy-setup__agents">
      <header class="deploy-setup__agents-header">
        <div class="deploy-setup__agents-heading">
          <h3>Connected agents</h3>
          <span class="deploy-setup__agents-count" data-testid="deploy-agents-count">{{ agents.length }}</span>
        </div>
        <AddAgentButton :project-id="projectId" :agents="agents" :reload="reload" @added="reload" @modal-state-change="$emit('modal-state-change', $event)" />
      </header>

      <div class="deploy-setup__agents-body">
        <AgentTable
          v-if="agents.length > 0"
          :agents="agents"
          :show-actions="true"
          :show-console="false"
          @delete="handleDelete"
        />
        <div v-else class="deploy-setup__agents-empty">
          <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="2" width="20" height="8" rx="2" ry="2"/><rect x="2" y="14" width="20" height="8" rx="2" ry="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/></svg>
          <p class="deploy-setup__agents-empty-title">No agents registered for this project.</p>
          <p class="deploy-setup__agents-empty-hint">Use “Add agent” above to connect your first Harvest agent.</p>
        </div>
      </div>
    </section>

    <div v-if="deletingAgent" class="modal" @click.self="cancelDelete">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="cancelDelete">✕</button>
        <h3>Delete agent</h3>
        <p>
          Delete agent <strong>{{ deletingAgent.hostname || deletingAgent.id }}</strong>?
          <template v-if="deletingAgent.online"> This agent is currently <strong>online</strong>.</template>
          <template v-if="deletingAgent.provider === 'lxd'"> The LXD container backing this agent will also be deleted.</template>
          This cannot be undone.
        </p>
        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" @click="cancelDelete">Cancel</button>
          <button class="p-button--negative is-dense" type="button" :disabled="deleting" @click="confirmDelete">Delete</button>
        </div>
      </div>
    </div>

    <footer class="deploy-setup__footer">
      <p class="deploy-setup__footer-hint">You can always add agents on the Agents page.</p>
      <button
        class="p-button--positive deploy-setup__next-btn"
        type="button"
        data-testid="deploy-next-btn"
        @click="$emit('next')"
      >Next</button>
    </footer>
  </div>
</template>

<script setup>
import { ref } from 'vue';
import AgentTable from '../agents/AgentTable.vue';
import AddAgentButton from '../agents/AddAgentButton.vue';
import { deleteAgent } from '../../lib/api.js';

const props = defineProps({
  projectId: { type: String, required: true },
  agents:    { type: Array, default: () => [] },
  reload:    { type: Function, default: async () => {} },
});
defineEmits(['next', 'modal-state-change']);

const deletingAgent = ref(null);
const deleting      = ref(false);

function handleDelete(agent) {
  deletingAgent.value = agent;
}

function cancelDelete() {
  deletingAgent.value = null;
}

async function confirmDelete() {
  const agent = deletingAgent.value;
  if (!agent) return;
  deleting.value = true;
  try {
    await deleteAgent(props.projectId, agent.id);
    deletingAgent.value = null;
    await props.reload();
  } catch {
  } finally {
    deleting.value = false;
  }
}
</script>

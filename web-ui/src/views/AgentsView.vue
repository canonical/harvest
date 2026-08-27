<template>
  <div class="agents-view">
    <div class="page-header">
      <h2>Agents</h2>
      <AddAgentButton :project-id="projectId" :agents="agents" :reload="load" @added="load" />
    </div>

    <template v-if="agents.length > 0">
      <AgentTable :agents="agents" @delete="handleDelete" />
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
          <button class="p-button--base is-dense" type="button" @click="deletingAgent = null">Cancel</button>
          <button class="p-button--negative is-dense" type="button" :disabled="deleting" @click="confirmDelete">Delete</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue';
import { listProjectAgents, deleteAgent } from '../lib/api.js';
import AgentTable from '../components/agents/AgentTable.vue';
import AddAgentButton from '../components/agents/AddAgentButton.vue';

const props = defineProps({ projectId: { type: String, required: true } });

const agents        = ref([]);
const deletingAgent = ref(null);
const deleting      = ref(false);
let refreshTimer = null;

async function load() {
  try {
    agents.value = await listProjectAgents(props.projectId);
  } catch {}
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

onMounted(() => {
  load();
  refreshTimer = setInterval(load, 15_000);
});

onUnmounted(() => {
  clearInterval(refreshTimer);
});
</script>

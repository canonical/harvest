<template>
  <div class="deploy-setup" data-testid="deploy-setup">
    <div class="deploy-setup__intro">
      <h2 class="deploy-setup__title">Connect agents</h2>
      <p class="deploy-setup__lede">
        Connect at least one Harvest agent to run the deployment. Once an agent is
        available, generate the deployment artifacts.
      </p>
    </div>

    <div class="deploy-setup__agents">
      <div class="deploy-setup__agents-header">
        <h3>Connected agents</h3>
        <AddAgentButton :project-id="projectId" :agents="agents" :reload="reload" @added="reload" />
      </div>

      <AgentTable v-if="agents.length > 0" :agents="agents" :show-actions="false" />
      <p v-else class="agents-empty">No agents registered for this project.</p>
    </div>

    <div class="deploy-setup__footer">
      <button
        class="p-button--positive"
        type="button"
        data-testid="deploy-next-btn"
        @click="$emit('next')"
      >Next</button>
    </div>
  </div>
</template>

<script setup>
import AgentTable from '../agents/AgentTable.vue';
import AddAgentButton from '../agents/AddAgentButton.vue';

defineProps({
  projectId: { type: String, required: true },
  agents:    { type: Array, default: () => [] },
  reload:    { type: Function, default: async () => {} },
});
defineEmits(['next']);
</script>

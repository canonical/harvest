<template>
  <div class="deployment-cost-panel" v-if="cost">
    <h3 class="deployment-cost-panel__title">LLM cost</h3>
    <div class="deployment-cost-panel__total">
      <span class="deployment-cost-panel__total-label">Total</span>
      <span class="deployment-cost-panel__total-value">{{ cost.total.total_cost_display }}</span>
      <span class="deployment-cost-panel__total-calls">{{ cost.total.total_calls }} calls</span>
    </div>
    <table class="deployment-cost-panel__table" v-if="scopeRows.length">
      <thead>
        <tr><th>Scope</th><th>Cost</th><th>Calls</th><th>Tokens (in/out)</th></tr>
      </thead>
      <tbody>
        <tr v-for="row in scopeRows" :key="row.scope">
          <td>{{ row.label }}</td>
          <td>{{ row.summary.total_cost_display }}</td>
          <td>{{ row.summary.total_calls }}</td>
          <td>{{ formatTokens(row.summary.total_input_tokens) }} / {{ formatTokens(row.summary.total_output_tokens) }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { fetchDeploymentCost } from '../../lib/api.js';

const props = defineProps({
  projectId: { type: String, required: true },
  deploymentId: { type: String, required: true },
});

const cost = ref(null);

const SCOPE_LABELS = {
  design: 'Design document',
  provision: 'Deployment artifact',
  proposal: 'Change proposals',
  chat: 'Chat',
  title: 'Title generation',
  overview: 'Overview',
};

const scopeRows = computed(() => {
  if (!cost.value?.by_scope) return [];
  return Object.entries(cost.value.by_scope)
    .map(([scope, summary]) => ({ scope, label: SCOPE_LABELS[scope] ?? scope, summary }))
    .filter(r => r.summary.total_calls > 0)
    .sort((a, b) => b.summary.total_cost_microusd - a.summary.total_cost_microusd);
});

function formatTokens(n) {
  if (!n) return '0';
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

async function load() {
  if (!props.projectId || !props.deploymentId) return;
  try {
    cost.value = await fetchDeploymentCost(props.projectId, props.deploymentId);
  } catch {
    cost.value = null;
  }
}

watch(() => [props.projectId, props.deploymentId], () => load(), { immediate: true });
</script>

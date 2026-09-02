<template>
  <details class="tc-group" :class="{ 'tc-group--running': isRunning }">
    <summary class="tc-group__summary">
      <svg
        class="tc-group__summary-chevron"
        viewBox="0 0 10 10"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
        xmlns="http://www.w3.org/2000/svg"
        aria-hidden="true"
      >
        <polyline points="2,3 5,7 8,3"/>
      </svg>
      {{ summaryLabel }}
    </summary>
    <ToolCallStep v-for="(step, i) in items" :key="step.id ?? i" :step="step" />
  </details>
</template>

<script setup>
import { computed } from 'vue';
import ToolCallStep from './ToolCallStep.vue';

const props = defineProps({
  items: { type: Array, required: true },
});

const GROUP_NOUNS = {
  run_cypher:                     'graph queries',
  get_symbol_source:              'source lookups',
  get_file_symbols:               'file scans',
  search_symbols:                 'searches',
  find_callers:                   'caller traces',
  find_callees:                   'callee traces',
  get_imports:                    'import checks',
  compare_symbol_across_versions: 'version comparisons',
};

const isRunning = computed(() => props.items.some(i => i.status === 'running'));

const summaryLabel = computed(() => {
  const n = props.items.length;
  const name = props.items[0]?.name;
  const noun = GROUP_NOUNS[name] ?? name?.replace(/_/g, ' ') ?? 'steps';
  return `Ran ${n} ${noun}`;
});
</script>

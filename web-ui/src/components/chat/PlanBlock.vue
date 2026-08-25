<template>
  <div class="plan-block" :class="{ 'plan-block--sticky': sticky, 'plan-block--collapsed': collapsed }">
    <div class="plan-block__header" role="button" tabindex="0" @click="toggle" @keydown.enter.prevent="toggle">
      <svg class="plan-block__chevron" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" :style="collapsed ? '' : 'transform: rotate(180deg)'">
        <polyline points="2,3 5,7 8,3" />
      </svg>
      <span class="plan-block__title">Plan</span>
      <span class="plan-block__progress">{{ doneCount }}/{{ steps.length }}</span>
    </div>
    <ol v-if="!collapsed" class="plan-block__steps">
      <li v-for="(step, i) in steps" :key="i" class="plan-step" :class="`plan-step--${step.status || 'pending'}`">
        <span class="plan-step__icon" aria-hidden="true">{{ statusIcon(step.status) }}</span>
        <span class="plan-step__text">{{ step.text }}</span>
      </li>
    </ol>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';

const props = defineProps({
  steps:  { type: Array, required: true },
  sticky: { type: Boolean, default: false },
});

const collapsed = ref(false);

const doneCount = computed(() =>
  props.steps.filter(s => s.status === 'done').length
);

const allDone = computed(() =>
  props.steps.length > 0 && doneCount.value === props.steps.length
);

watch(allDone, (done) => {
  if (done) collapsed.value = true;
});

function toggle() {
  collapsed.value = !collapsed.value;
}

function statusIcon(status) {
  switch (status) {
    case 'done':     return '✓';
    case 'running':  return '◷';
    default:         return '○';
  }
}
</script>

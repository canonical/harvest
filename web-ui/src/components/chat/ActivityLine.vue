<template>
  <div class="activity-line" :class="`activity-line--${activity.state}`">
    <span class="activity-line__icon" aria-hidden="true"></span>
    <span class="activity-line__text" role="status" aria-live="polite">
      <Transition name="activity-line-slide">
        <span :key="lineKey" class="activity-line__line" :class="{ 'activity-line__line--shimmer': isWorking }">
          {{ activity.text }}
        </span>
      </Transition>
    </span>
    <button
      v-if="steps > 0"
      type="button"
      class="activity-line__toggle"
      :aria-expanded="String(expanded)"
      @click="emit('toggle')"
    >
      {{ isSettled ? (expanded ? 'Hide steps' : 'Show steps') : stepLabel(steps) }}
      <svg
        class="activity-line__chevron"
        viewBox="0 0 10 10"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <polyline points="2,3 5,7 8,3"/>
      </svg>
    </button>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ACTIVITY_STATE, isContinuation, stepLabel } from '../../lib/agent-activity.js';

const props = defineProps({
  activity: { type: Object, required: true },
  steps:    { type: Number, default: 0 },
  expanded: { type: Boolean, default: false },
});

const emit = defineEmits(['toggle']);

const lineKey = ref(0);

const isWorking = computed(() => props.activity.state === ACTIVITY_STATE.WORKING);
const isSettled = computed(() =>
  props.activity.state === ACTIVITY_STATE.DONE || props.activity.state === ACTIVITY_STATE.STOPPED
);

watch(() => props.activity.text, (next, previous) => {
  if (!isContinuation(previous, next)) lineKey.value += 1;
});
</script>

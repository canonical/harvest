<template>
  <div class="activity-line" :class="`activity-line--${activity.state}`">
    <component
      :is="canExpand ? 'button' : 'div'"
      :type="canExpand ? 'button' : undefined"
      class="activity-line__row"
      :aria-expanded="canExpand ? String(expanded) : undefined"
      @click="toggle"
    >
      <span class="activity-line__icon" aria-hidden="true"></span>
      <span class="activity-line__text">
        <Transition name="activity-line-slide">
          <span :key="lineKey" class="activity-line__line" :class="{ 'activity-line__line--shimmer': isWorking }">
            {{ activity.text }}
          </span>
        </Transition>
      </span>
      <span v-if="elapsed" class="activity-line__elapsed">{{ elapsed }}</span>
      <span v-if="canExpand" class="activity-line__steps">
        {{ stepsLabel }}
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
      </span>
    </component>
    <span class="activity-line__announcer" role="status" aria-live="polite">{{ activity.text }}</span>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ACTIVITY_STATE, isContinuation, stepLabel } from '../../lib/agent-activity.js';

const props = defineProps({
  activity: { type: Object, required: true },
  steps:    { type: Number, default: 0 },
  expanded: { type: Boolean, default: false },
  elapsed:  { type: String, default: null },
});

const emit = defineEmits(['toggle']);

const lineKey = ref(0);

const canExpand = computed(() => props.steps > 0);
const isWorking = computed(() => props.activity.state === ACTIVITY_STATE.WORKING);
const isSettled = computed(() =>
  props.activity.state === ACTIVITY_STATE.DONE || props.activity.state === ACTIVITY_STATE.STOPPED
);

const stepsLabel = computed(() => {
  if (!isSettled.value) return stepLabel(props.steps);
  return props.expanded ? 'Hide steps' : 'Show steps';
});

function toggle() {
  if (canExpand.value) emit('toggle');
}

watch(() => props.activity.text, (next, previous) => {
  if (!isContinuation(previous, next)) lineKey.value += 1;
});
</script>

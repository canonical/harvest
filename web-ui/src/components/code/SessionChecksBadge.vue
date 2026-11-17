<template>
  <button
    v-if="badge"
    type="button"
    class="p-chip checks-badge"
    :class="`checks-badge--${badge.kind}`"
    title="Show the checks in the Jobs tab"
    @click="emit('open')"
  >
    <span class="checks-badge__dot" aria-hidden="true" />
    <span class="p-chip__value">{{ badge.label }}</span>
  </button>
</template>

<script setup>
import { computed } from 'vue';
import { checksBadge } from '../../lib/session-jobs.js';
import { useSessionJobs } from '../../composables/useSessionJobs.js';

const props = defineProps({
  sessionId: { type: String, required: true },
});
const emit = defineEmits(['open']);

const { jobs } = useSessionJobs(props.sessionId);
const badge = computed(() => checksBadge(jobs.value));
</script>

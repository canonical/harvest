<template>
  <button
    v-if="isInResults"
    type="button"
    class="cs-chip"
    :title="`${location.file}:${location.line}`"
    @click="emit('select', location)"
  >
    {{ location.name }}
  </button>
  <span v-else class="cs-chip cs-chip--outside" :title="`${location.file}:${location.line} · not in these results`">
    {{ location.name }}
  </span>
</template>

<script setup>
import { computed } from 'vue';
import { itemIndexAt } from '../../lib/code-search-view.js';

const props = defineProps({
  location: { type: Object, required: true },
  view:     { type: Object, required: true },
});

const emit = defineEmits(['select']);

const isInResults = computed(() => itemIndexAt(props.view, props.location) >= 0);
</script>

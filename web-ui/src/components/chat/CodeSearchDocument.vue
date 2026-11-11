<template>
  <div class="cs-detail">
    <div class="cs-detail__title">
      <span class="cs-kind cs-kind--doc">doc</span>
      <span class="cs-detail__name">{{ documentation.name }}</span>
      <span class="cs-detail__location">{{ locationLabel(documentation) }}</span>
      <span class="cs-badge cs-badge--unverified">Not verified</span>
    </div>
    <p class="cs-note">Documentation can be outdated or describe plans. The answer relies on the code.</p>
    <div class="cs-excerpt">{{ documentation.excerpt }}</div>
    <div class="cs-links">
      <span class="cs-links__label">Points to</span>
      <template v-if="documentation.points_to.length">
        <CodeSearchLink
          v-for="target in documentation.points_to"
          :key="`${target.file}:${target.name}`"
          :location="target"
          :view="view"
          @select="emit('select', $event)"
        />
      </template>
      <span v-else class="cs-links__empty">no code it names was found</span>
    </div>
  </div>
</template>

<script setup>
import CodeSearchLink from './CodeSearchLink.vue';
import { locationLabel } from '../../lib/code-search-view.js';

defineProps({
  documentation: { type: Object, required: true },
  view:          { type: Object, required: true },
});

const emit = defineEmits(['select']);
</script>

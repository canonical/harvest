<template>
  <div class="pr-block" :class="{ 'pr-block--merging': block.merging }">
    <div class="pr-block__header">
      <svg class="pr-block__icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" xmlns="http://www.w3.org/2000/svg">
        <circle cx="4" cy="3" r="1.6" />
        <circle cx="4" cy="13" r="1.6" />
        <circle cx="12.5" cy="8" r="1.6" />
        <path d="M4 4.6v6.8M4 8h4.5c1.1 0 1.5-.3 2-1l1-1.4" />
      </svg>
      <span class="pr-block__title">{{ headerText }}</span>
    </div>

    <div class="pr-block__lanes">
      <div
        v-for="(lead, i) in block.leads"
        :key="i"
        class="pr-lane"
        :class="{ 'pr-lane--open': openLanes[i] }"
      >
        <div
          class="pr-lane__row"
          :class="{ 'pr-lane__row--clickable': !!lead.preview }"
          :role="lead.preview ? 'button' : undefined"
          :tabindex="lead.preview ? 0 : undefined"
          :aria-expanded="lead.preview ? String(!!openLanes[i]) : undefined"
          @click="lead.preview && toggle(i)"
          @keydown.enter.prevent="lead.preview && toggle(i)"
          @keydown.space.prevent="lead.preview && toggle(i)"
        >
          <svg v-if="lead.status === 'running'" class="pr-lane__status pr-lane__status--running" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true" xmlns="http://www.w3.org/2000/svg">
            <path d="M.81 7.36a1.92 1.92 0 1 1 3.799.572A1.92 1.92 0 0 1 .81 7.36M8.826 3.033a1.92 1.92 0 1 1 3.755.806 1.92 1.92 0 0 1-3.755-.806M7.04 12.585a4.68 4.68 0 0 1-3.19-2.432 2.76 2.76 0 0 1-1.64.202 6.25 6.25 0 0 0 4.498 3.77c.45.098.908.144 1.364.141a2.74 2.74 0 0 1-.562-1.605 5 5 0 0 1-.47-.076M8.394 12.193a1.92 1.92 0 0 1 3.754.805 1.92 1.92 0 1 1-3.754-.805M12.943 11.89a6.3 6.3 0 0 0 1.22-2.587 6.3 6.3 0 0 0-.905-4.782 2.77 2.77 0 0 1-1.08 1.265 4.7 4.7 0 0 1-.154 4.674c.45.37.77.87.919 1.43M2.56 4.892a2.75 2.75 0 0 1 1.603.41 4.68 4.68 0 0 1 3.77-2.015q.012-.218.057-.433c.088-.411.268-.795.525-1.124A6.31 6.31 0 0 0 2.56 4.892"/>
          </svg>
          <svg v-else class="pr-lane__status pr-lane__status--done" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" xmlns="http://www.w3.org/2000/svg">
            <path d="M3 8.5l3 3 7-7" />
          </svg>

          <span class="pr-lane__label" :title="lead.label">{{ lead.label }}</span>

          <span v-if="lead.status === 'done'" class="pr-lane__stats">
            {{ formatDuration(lead.durationMs) }}<template v-if="lead.iterations"> · {{ lead.iterations }} {{ lead.iterations === 1 ? 'round' : 'rounds' }}</template>
          </span>

          <svg
            v-if="lead.preview"
            class="pr-lane__chevron"
            viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            aria-hidden="true" xmlns="http://www.w3.org/2000/svg"
            :style="openLanes[i] ? 'transform: rotate(180deg)' : ''"
          >
            <polyline points="2,3 5,7 8,3" />
          </svg>
        </div>

        <div v-if="lead.preview && openLanes[i]" class="pr-lane__detail">{{ lead.preview }}</div>
      </div>
    </div>

    <div v-if="block.merging" class="pr-block__merge">
      <svg class="pr-block__merge-arrow" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" xmlns="http://www.w3.org/2000/svg">
        <polyline points="2,3 5,7 8,3" />
      </svg>
      Merging findings into one answer…
    </div>
  </div>
</template>

<script setup>
import { reactive, computed } from 'vue';
import { formatDuration } from '../../lib/utils.js';

const props = defineProps({
  block: { type: Object, required: true },
});

const openLanes = reactive({});

function toggle(i) {
  openLanes[i] = !openLanes[i];
}

const headerText = computed(() => {
  const n = props.block.leads.length;
  if (!props.block.merging) return `Investigating ${n} leads in parallel`;
  return `Investigated ${n} leads in parallel · ${formatDuration(props.block.totalDurationMs)}`;
});
</script>

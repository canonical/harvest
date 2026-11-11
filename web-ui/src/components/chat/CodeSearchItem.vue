<template>
  <div class="cs-detail">
    <div class="cs-detail__title">
      <span class="cs-kind" :class="`cs-kind--${item.kind}`">{{ item.kind }}</span>
      <span class="cs-detail__name">{{ displayName(item) }}</span>
      <span v-if="location" class="cs-detail__location">{{ location }}</span>
      <span v-else-if="isTable" class="cs-detail__location">{{ rows.length }} row{{ rows.length === 1 ? '' : 's' }}</span>
      <span v-if="item.language" class="cs-badge">{{ item.language }}</span>
      <button v-if="location" type="button" class="cs-action" @click="copyCitation">
        {{ copied ? 'Copied' : 'Copy citation' }}
      </button>
    </div>
    <div v-if="item.owner" class="cs-links">
      <span class="cs-links__label">Member of</span>
      <CodeSearchLink :location="item.owner" :view="view" @select="emit('select', $event)" />
    </div>
    <span v-if="isModule" class="cs-section-label">Outline</span>
    <CodeSearchTable v-if="rows.length" :rows="rows" :view="view" @select="emit('select', $event)" />
    <pre v-else-if="item.code" class="cs-code" :class="{ 'cs-code--plain': !numbered }"><code><span
      v-for="line in lines"
      :key="line.number"
      class="cs-code__line"
      :data-line="numbered ? line.number : null"
      v-html="line.html"
    ></span></code></pre>
    <p v-else class="cs-note">The search found nothing for this {{ item.kind }}.</p>
    <p v-if="item.truncated" class="cs-note">Showing the first part of this {{ item.kind }}.</p>
    <div v-if="recentChanges.length" class="cs-changes">
      <span class="cs-section-label">Recent changes</span>
      <ul class="cs-changes__list">
        <li v-for="(change, index) in recentChanges" :key="index">{{ change }}</li>
      </ul>
    </div>
    <div v-if="item.calls.length" class="cs-links">
      <span class="cs-links__label">Calls</span>
      <CodeSearchLink
        v-for="call in item.calls"
        :key="`${call.file}:${call.name}`"
        :location="call"
        :view="view"
        @select="emit('select', $event)"
      />
    </div>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import CodeSearchLink from './CodeSearchLink.vue';
import CodeSearchTable from './CodeSearchTable.vue';
import { citation, displayName, hasSourceLines, highlightedLines, locationLabel } from '../../lib/code-search-view.js';

const COPIED_MS = 1200;

const props = defineProps({
  item: { type: Object, required: true },
  view: { type: Object, required: true },
});

const emit = defineEmits(['select']);

const copied = ref(false);

const location = computed(() => locationLabel(props.item));
const rows = computed(() => props.item.rows ?? []);
const isTable = computed(() => props.item.kind === 'table');
const isModule = computed(() => props.item.kind === 'module');
const recentChanges = computed(() => props.item.recent_changes ?? []);
const numbered = computed(() => hasSourceLines(props.item));
const lines = computed(() =>
  highlightedLines(props.item.code, props.item.file || 'table.txt', props.item.start_line || 1)
);

watch(() => props.item, () => { copied.value = false; });

async function copyCitation() {
  try {
    await navigator.clipboard.writeText(citation(props.view, props.item));
  } catch {
    return;
  }
  copied.value = true;
  setTimeout(() => { copied.value = false; }, COPIED_MS);
}
</script>

<template>
  <div v-if="llm.providers.length" ref="rootEl" class="llm-picker">
    <button
      type="button"
      class="llm-picker__trigger"
      :aria-expanded="open"
      @click="toggle"
    ><span class="llm-picker__trigger-icon" aria-hidden="true">+</span>{{ triggerLabel }}</button>

    <div v-if="open" class="llm-picker__panel" role="listbox">
      <input
        ref="searchInput"
        v-model="filterText"
        type="text"
        class="llm-picker__search"
        placeholder="Search models…"
        @keydown.down.prevent="move(1)"
        @keydown.up.prevent="move(-1)"
        @keydown.enter.prevent="chooseHighlighted"
        @keydown.esc="close"
      />
      <ul class="llm-picker__list">
        <li
          v-for="(entry, i) in filteredEntries"
          :key="entry.key"
          class="llm-picker__option llm-picker__option--model"
          :class="{ 'llm-picker__option--active': i === highlight }"
          @mouseenter="highlight = i"
          @click="choose(entry)"
        >
          <span class="llm-picker__option-model">{{ entry.label }}</span>
          <span class="llm-picker__option-kind">{{ entry.providerName }}</span>
        </li>
        <li v-if="!filteredEntries.length" class="llm-picker__empty">No models match "{{ filterText }}"</li>
      </ul>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, nextTick } from 'vue';
import { useLlmStore } from '../../stores/llm.js';

const llm = useLlmStore();

const open = ref(false);
const filterText = ref('');
const highlight = ref(-1);
const rootEl = ref(null);
const searchInput = ref(null);

const entries = computed(() =>
  llm.providers.flatMap(p =>
    (p.models ?? []).map(m => ({
      key: `${p.id}:${m.id}`,
      providerId: p.id,
      modelId: m.id,
      providerName: p.name || p.kind,
      label: m.display_name || m.id,
    }))
  )
);

const filteredEntries = computed(() => {
  const q = filterText.value.trim().toLowerCase();
  if (!q) return entries.value;
  return entries.value.filter(e =>
    e.label.toLowerCase().includes(q)
    || e.providerName.toLowerCase().includes(q)
    || e.modelId.toLowerCase().includes(q)
  );
});

const triggerLabel = computed(() => {
  if (!llm.selection?.providerId) return 'Auto';
  const match = entries.value.find(e =>
    e.providerId === llm.selection.providerId && e.modelId === llm.selection.model
  );
  return match ? match.label : (llm.selection.model || llm.selection.providerId);
});

function toggle() {
  open.value = !open.value;
  if (open.value) {
    filterText.value = '';
    highlight.value = -1;
    nextTick(() => searchInput.value?.focus());
  }
}

function close() {
  open.value = false;
}

function choose(entry) {
  llm.setSelection(entry.providerId, entry.modelId);
  close();
}

function move(delta) {
  const max = filteredEntries.value.length - 1;
  highlight.value = Math.min(max, Math.max(-1, highlight.value + delta));
}

function chooseHighlighted() {
  const entry = filteredEntries.value[highlight.value];
  if (entry) choose(entry);
}

function onDocumentMousedown(e) {
  if (open.value && rootEl.value && !rootEl.value.contains(e.target)) close();
}

onMounted(() => document.addEventListener('mousedown', onDocumentMousedown));
onUnmounted(() => document.removeEventListener('mousedown', onDocumentMousedown));
</script>

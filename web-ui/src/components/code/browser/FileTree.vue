<template>
  <ul ref="root" class="file-tree" role="tree" :aria-label="label" @keydown="onKeydown">
    <li v-if="error" class="file-tree__message code-error" role="alert">{{ error }}</li>
    <li
      v-for="row in rows"
      :key="row.entry.path"
      class="file-tree__item"
      role="treeitem"
      :data-path="row.entry.path"
      :aria-level="row.depth + 1"
      :aria-expanded="isDirectory(row.entry) ? String(row.isOpen) : undefined"
      :aria-selected="String(row.entry.path === selectedPath)"
      :tabindex="row.entry.path === focusedPath ? 0 : -1"
      :style="{ '--depth': row.depth }"
      @click="activate(row)"
      @focus="focusedPath = row.entry.path"
    >
      <i v-if="isDirectory(row.entry)" class="file-tree__chevron" :class="row.isOpen ? 'p-icon--chevron-down' : 'p-icon--chevron-right'" />
      <span v-else class="file-tree__chevron" />
      <i :class="isDirectory(row.entry) ? 'p-icon--folder' : 'p-icon--file'" />
      <span class="file-tree__name">{{ row.entry.name }}</span>
      <i v-if="row.isLoading" class="p-icon--spinner u-animation--spin file-tree__loading" />
    </li>
  </ul>
</template>

<script setup>
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue';
import { ROOT, ancestorsOf, isDirectory, parentOf, visibleRows } from '../../../lib/repository-browser.js';

const props = defineProps({
  label: { type: String, required: true },
  loadChildren: { type: Function, required: true },
  selectedPath: { type: String, default: ROOT },
});
const emit = defineEmits(['select']);

const root = ref(null);
const childrenByPath = reactive(new Map());
const expanded = reactive(new Set());
const focusedPath = ref(null);
const error = ref('');

const rows = computed(() => visibleRows(childrenByPath, expanded));

async function load(path) {
  if (childrenByPath.has(path)) return;
  try {
    childrenByPath.set(path, await props.loadChildren(path));
  } catch (e) {
    error.value = e.message;
  }
}

function expand(path) {
  expanded.add(path);
  return load(path);
}

function toggle(entry) {
  if (expanded.has(entry.path)) expanded.delete(entry.path);
  else expand(entry.path);
}

function activate(row) {
  focusedPath.value = row.entry.path;
  if (isDirectory(row.entry)) toggle(row.entry);
  emit('select', row.entry);
}

async function focusRow(path) {
  focusedPath.value = path;
  await nextTick();
  const items = root.value?.querySelectorAll('[role="treeitem"]') ?? [];
  [...items].find((item) => item.dataset.path === path)?.focus();
}

function moveFocus(step) {
  const index = rows.value.findIndex((row) => row.entry.path === focusedPath.value);
  const next = rows.value[Math.min(Math.max(index + step, 0), rows.value.length - 1)];
  if (next) focusRow(next.entry.path);
}

const KEY_ACTIONS = {
  ArrowDown: () => moveFocus(1),
  ArrowUp: () => moveFocus(-1),
  Home: () => moveFocus(-Infinity),
  End: () => moveFocus(Infinity),
  ArrowRight: (row) => {
    if (!isDirectory(row.entry)) return;
    if (row.isOpen) moveFocus(1);
    else expand(row.entry.path);
  },
  ArrowLeft: (row) => {
    if (row.isOpen) expanded.delete(row.entry.path);
    else if (parentOf(row.entry.path) !== ROOT) focusRow(parentOf(row.entry.path));
  },
  Enter: (row) => activate(row),
  ' ': (row) => activate(row),
};

function onKeydown(event) {
  const action = KEY_ACTIONS[event.key];
  const row = rows.value.find((candidate) => candidate.entry.path === focusedPath.value);
  if (!action || !row) return;
  event.preventDefault();
  action(row);
}

async function reveal(path) {
  await Promise.all(ancestorsOf(path).map(expand));
  focusedPath.value = path || rows.value[0]?.entry.path || null;
}

watch(() => props.selectedPath, reveal);

onMounted(async () => {
  await load(ROOT);
  await reveal(props.selectedPath);
});
</script>

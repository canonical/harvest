<template>
  <div class="terminal-view">
    <form v-if="searching" class="terminal-view__search" role="search" @submit.prevent="findNext">
      <input ref="searchInput" v-model="query" type="search" aria-label="Find in terminal" placeholder="Find" @keydown.esc.prevent="closeSearch" />
      <button class="p-button--base is-dense" type="button" @click="findPrevious">Previous</button>
      <button class="p-button--base is-dense" type="submit">Next</button>
      <button class="p-button--base is-dense" type="button" @click="closeSearch">Close</button>
    </form>
    <div ref="host" class="terminal-view__host" />
  </div>
</template>

<script setup>
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { loadAddons, loadRenderer } from '../../lib/terminal-addons.js';
import { TERMINAL_FONT, terminalTheme } from '../../lib/terminal-theme.js';
import { onThemeChange } from '../../lib/theme.js';
import { useTerminalSocket } from '../../composables/useTerminalSocket.js';

const props = defineProps({
  urlFor: { type: Function, required: true },
});
const emit = defineEmits(['status', 'title']);

const host = ref(null);
const searchInput = ref(null);
const searching = ref(false);
const query = ref('');

const terminal = new Terminal({
  allowProposedApi: true,
  cursorBlink: true,
  fontFamily: TERMINAL_FONT,
  fontSize: 14,
  scrollback: 10_000,
  theme: terminalTheme(),
});
const { fit, search } = loadAddons(terminal);
const connection = useTerminalSocket({
  urlFor: () => props.urlFor({ cols: terminal.cols, rows: terminal.rows }),
  onData: (bytes) => terminal.write(bytes),
});

let resizing = null;

function passesToHarvest(event) {
  return event.ctrlKey && event.altKey;
}

function opensSearch(event) {
  return event.type === 'keydown' && event.ctrlKey && event.shiftKey && event.key.toLowerCase() === 'f';
}

async function openSearch() {
  searching.value = true;
  await nextTick();
  searchInput.value?.focus();
}

function closeSearch() {
  searching.value = false;
  search.clearDecorations();
  terminal.focus();
}

function findNext() {
  if (query.value) search.findNext(query.value);
}

function findPrevious() {
  if (query.value) search.findPrevious(query.value);
}

function restart() {
  terminal.reset();
  connection.restart();
}

terminal.attachCustomKeyEventHandler((event) => {
  if (opensSearch(event)) {
    openSearch();
    return false;
  }
  return !passesToHarvest(event);
});
terminal.onData((data) => connection.send(data));
terminal.onResize(({ cols, rows }) => connection.resize(cols, rows));
terminal.onTitleChange((title) => emit('title', title));

watch(connection.state, (state) => {
  emit('status', state);
  if (state.status === 'open') {
    connection.resize(terminal.cols, terminal.rows);
    terminal.focus();
  }
}, { immediate: true });

const stopFollowingTheme = onThemeChange(() => {
  terminal.options.theme = terminalTheme();
});

onMounted(() => {
  terminal.open(host.value);
  loadRenderer(terminal);
  fit.fit();
  resizing = new ResizeObserver(() => fit.fit());
  resizing.observe(host.value);
  connection.open();
});

onUnmounted(() => {
  resizing?.disconnect();
  stopFollowingTheme();
  connection.dispose();
  terminal.dispose();
});

defineExpose({ restart, focus: () => terminal.focus() });
</script>

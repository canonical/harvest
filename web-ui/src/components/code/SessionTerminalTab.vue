<template>
  <div class="session-tab session-terminal">
    <div v-if="notice" :class="`p-notification--${notice.tone} session-terminal__notice`">
      <div class="p-notification__content">
        <p class="p-notification__message">{{ notice.message }}</p>
      </div>
      <div v-if="notice.action" class="p-notification__actions">
        <button class="p-button--base is-dense u-no-margin--bottom" type="button" @click="view?.restart()">{{ notice.action }}</button>
      </div>
    </div>
    <p v-if="error" class="code-error session-terminal__error" role="alert">{{ error }}</p>
    <TerminalView v-if="terminal" ref="view" :key="terminal.id" :url-for="urlFor" @status="status = $event" @title="onTitle" />
    <div v-else-if="ended" class="session-tab__empty">
      <p>This terminal has ended.</p>
      <button class="p-button--positive is-dense" type="button" @click="attachNewTerminal">New terminal</button>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref, watch } from 'vue';
import TerminalView from '../terminal/TerminalView.vue';
import { useLayoutWorkspace } from '../../lib/layout-workspace.js';
import { terminalSocketUrl } from '../../lib/code-api.js';
import { connecting, connectionNotice } from '../../lib/terminal-connection.js';
import { confirm } from '../../composables/useConfirm.js';

const props = defineProps({
  tab: { type: Object, required: true },
});

const workspace = useLayoutWorkspace();
const view = ref(null);
const status = ref(connecting());
const error = ref('');

const terminal = computed(() => workspace.terminals.find(t => t.id === props.tab.terminalId) ?? null);
const ended = computed(() => Boolean(props.tab.terminalId) && !terminal.value);
const notice = computed(() => (terminal.value ? connectionNotice(status.value) : null));

function urlFor(size) {
  return terminalSocketUrl(workspace.sessionId, props.tab.terminalId, size);
}

function onTitle(title) {
  workspace.setLiveTitle(props.tab.tabId, title);
}

async function attachNewTerminal() {
  error.value = '';
  try {
    const created = await workspace.newTerminal();
    workspace.updateTab(props.tab.tabId, { terminalId: created.id, title: created.title });
  } catch (e) {
    error.value = e.message;
  }
}

async function endTerminal() {
  const confirmed = await confirm({
    title: `End ${terminal.value.title}?`,
    message: 'Its shell and everything still running in it stop. Other terminals of this session keep running.',
    confirmLabel: 'End terminal',
    danger: true,
  });
  if (!confirmed) return;
  try {
    await workspace.endTerminal(terminal.value.id);
  } catch (e) {
    error.value = e.message;
  }
}

watch(() => workspace.tabRequest, (request) => {
  if (request?.tabId !== props.tab.tabId) return;
  if (request.action === 'end-terminal' && terminal.value) endTerminal();
});

onMounted(() => {
  if (!props.tab.terminalId) attachNewTerminal();
});
</script>

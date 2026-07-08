<template>
  <div class="agent-console-view">
    <div class="console-header">
      <div class="console-header__left">
        <button id="console-back-btn" class="console-icon-btn" type="button" aria-label="Back to agents" title="Back to agents" @click="goBack">
          <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="19" y1="12" x2="5" y2="12"/><polyline points="12 19 5 12 12 5"/></svg>
        </button>
        <div class="console-header__info">
          <h2 class="console-title">{{ agent ? (agent.hostname || agent.id) : agentId }}</h2>
          <span
            v-if="agent"
            class="console-status"
            :class="agent.online ? 'console-status--online' : 'console-status--offline'"
          >{{ agent.online ? 'Online' : 'Offline' }}</span>
        </div>
      </div>
      <div class="console-actions">
        <button
          id="console-power-btn"
          class="console-icon-btn"
          type="button"
          :disabled="!isLxd || actionInProgress"
          :title="isLxd ? (isRunning ? 'Stop' : 'Start') : 'Only available for Harvest-managed (LXD) agents'"
          :aria-label="isRunning ? 'Stop agent' : 'Start agent'"
          @click="handlePower"
        >
          <svg v-if="isRunning" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/></svg>
          <svg v-else xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="5 3 19 12 5 21 5 3"/></svg>
        </button>
        <button
          id="console-restart-btn"
          class="console-icon-btn"
          type="button"
          :disabled="!isLxd || actionInProgress"
          :title="isLxd ? 'Restart' : 'Only available for Harvest-managed (LXD) agents'"
          aria-label="Restart agent"
          @click="handleRestart"
        >
          <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>
        </button>
        <button
          id="console-delete-btn"
          class="console-icon-btn console-icon-btn--danger"
          type="button"
          title="Delete agent"
          aria-label="Delete agent"
          @click="showDeleteConfirm = true"
        >
          <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/></svg>
        </button>
      </div>
    </div>

    <p v-if="actionError" class="console-action-error">{{ actionError }}</p>

    <div class="console-terminal">
      <div ref="terminalContainer" class="console-terminal__inner"></div>
    </div>

    <div v-if="showDeleteConfirm" id="console-delete-modal" class="modal" @click.self="showDeleteConfirm = false">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="showDeleteConfirm = false">✕</button>
        <h3>Delete agent</h3>
        <p>
          Delete agent <strong>{{ agent?.hostname || agentId }}</strong>?
          <template v-if="isLxd"> The LXD container backing this agent will also be deleted.</template>
          This cannot be undone.
        </p>
        <div class="modal-actions">
          <button class="p-button--base" type="button" @click="showDeleteConfirm = false">Cancel</button>
          <button id="console-delete-confirm-btn" class="p-button--negative" type="button" :disabled="deleting" @click="confirmDelete">Delete</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, nextTick } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import {
  listProjectAgents, deleteAgent, startAgent, stopAgent, restartAgent, consoleSocketUrl,
} from '../lib/api.js';
import { useConsoleSocket } from '../composables/useConsoleSocket.js';

const STATUS_POLL_MS = 3000;

const props = defineProps({ projectId: { type: String, required: true } });

const route  = useRoute();
const router = useRouter();
const agentId = route.params.agentId;

const agent             = ref(null);
const isLxd              = computed(() => agent.value?.provider === 'lxd');
const isRunning          = computed(() => agent.value?.online === true);
const actionInProgress   = ref(false);
const actionError        = ref('');
const showDeleteConfirm  = ref(false);
const deleting           = ref(false);
const terminalContainer  = ref(null);
const consoleStatus      = ref('connecting');

let terminal          = null;
let fitAddon           = null;
let resizeObserver     = null;
let statusPollTimer    = null;
let shouldAutoConnect  = true;
let unmounted          = false;
const socket = useConsoleSocket();

function terminalTheme() {
  const styles = getComputedStyle(document.documentElement);
  return {
    background: styles.getPropertyValue('--bg-code').trim() || '#0d0d0d',
    foreground: '#f0f0f0',
  };
}

function writeNotice(text, color = '90') {
  terminal.write(`\r\n\x1b[${color}m[${text}]\x1b[0m\r\n`);
}

function handleControl(msg) {
  if (msg.type === 'ready') {
    consoleStatus.value = 'open';
  } else if (msg.type === 'exited') {
    const suffix = msg.code != null ? ` with code ${msg.code}` : '';
    writeNotice(`process exited${suffix} — press any key to start a new session`, '33');
    consoleStatus.value = 'ended';
  } else if (msg.type === 'error') {
    writeNotice(msg.message || 'console error', '31');
    consoleStatus.value = 'ended';
  }
}

function startConsole() {
  if (!isRunning.value) {
    consoleStatus.value = 'offline';
    return;
  }
  socket.close();
  consoleStatus.value = 'connecting';
  socket.connect(consoleSocketUrl(props.projectId, agentId, terminal.cols, terminal.rows), {
    onData:    (bytes) => terminal.write(bytes),
    onControl: handleControl,
    onClose:   () => {
      if (consoleStatus.value !== 'offline') {
        consoleStatus.value = 'ended';
      }
    },
  });
}

async function initTerminal() {
  terminal = new Terminal({
    convertEol: true,
    fontFamily: "'Ubuntu Mono', 'Courier New', monospace",
    fontSize: 13,
    theme: terminalTheme(),
  });
  fitAddon = new FitAddon();
  terminal.loadAddon(fitAddon);
  terminal.open(terminalContainer.value);
  fitAddon.fit();

  terminal.onData((data) => {
    if (consoleStatus.value === 'ended' && isRunning.value) {
      startConsole();
      return;
    }
    socket.send(data);
  });

  resizeObserver = new ResizeObserver(() => {
    fitAddon.fit();
    socket.resize(terminal.cols, terminal.rows);
  });
  resizeObserver.observe(terminalContainer.value);

  if (isRunning.value) {
    startConsole();
  } else {
    consoleStatus.value = 'offline';
    writeNotice('agent is offline');
  }
}

function goBack() {
  router.push('/agents');
}

async function refreshAgent() {
  try {
    const agents = await listProjectAgents(props.projectId);
    agent.value = agents.find(a => a.id === agentId) || null;
  } catch {
  }
}

async function pollAgentStatus() {
  const wasOnline = isRunning.value;
  await refreshAgent();
  if (unmounted) return;
  const nowOnline = isRunning.value;

  if (!wasOnline && nowOnline && shouldAutoConnect) {
    writeNotice('agent is back online, starting a new session');
    startConsole();
  } else if (wasOnline && !nowOnline && consoleStatus.value !== 'offline') {
    socket.close();
    consoleStatus.value = 'offline';
    writeNotice('agent disconnected', '31');
  }
}

function scheduleStatusPoll() {
  statusPollTimer = setTimeout(async () => {
    await pollAgentStatus();
    if (!unmounted) scheduleStatusPoll();
  }, STATUS_POLL_MS);
}

async function runAction(fn, afterSuccess) {
  actionInProgress.value = true;
  actionError.value = '';
  try {
    await fn();
    await refreshAgent();
    afterSuccess?.();
  } catch (e) {
    actionError.value = e.message || 'Action failed';
  } finally {
    actionInProgress.value = false;
  }
}

const handlePower = () => {
  if (isRunning.value) {
    shouldAutoConnect = false;
    return runAction(() => stopAgent(props.projectId, agentId), () => {
      socket.close();
      consoleStatus.value = 'offline';
      writeNotice('agent stopped');
    });
  }
  shouldAutoConnect = true;
  return runAction(() => startAgent(props.projectId, agentId), () => {
    writeNotice('starting agent…');
  });
};

const handleRestart = () => {
  shouldAutoConnect = true;
  return runAction(() => restartAgent(props.projectId, agentId), () => {
    socket.close();
    consoleStatus.value = 'offline';
    writeNotice('restarting agent…');
  });
};

async function confirmDelete() {
  deleting.value = true;
  try {
    await deleteAgent(props.projectId, agentId);
    router.push('/agents');
  } catch (e) {
    actionError.value = e.message || 'Failed to delete agent';
  } finally {
    deleting.value = false;
    showDeleteConfirm.value = false;
  }
}

onMounted(async () => {
  await refreshAgent();
  await nextTick();
  await initTerminal();
  scheduleStatusPoll();
});

onUnmounted(() => {
  unmounted = true;
  clearTimeout(statusPollTimer);
  socket.close();
  resizeObserver?.disconnect();
  terminal?.dispose();
});
</script>

<template>
  <div class="issue-chat" data-testid="issue-chat">
    <div class="issue-chat__messages">
      <ChatMessage v-for="(m, i) in messages" :key="i" :msg="m" :is-last="i === messages.length - 1" />
    </div>

    <div v-if="showProposedDiff" class="issue-chat__proposed-solution" data-testid="issue-chat-proposed-diff">
      <p class="issue-chat__proposed-summary">{{ proposedSummary }}</p>
      <DiffView :before="beforeFiles" :after="proposedFiles" />
      <button
        class="p-button--positive is-dense"
        data-testid="issue-chat-approve-btn"
        type="button"
        @click="$emit('approve')"
      >Approve</button>
    </div>

    <div v-if="error" class="p-notification--negative">
      <div class="p-notification__content"><p class="p-notification__message">{{ error }}</p></div>
    </div>

    <div class="form-group issue-chat__composer">
      <textarea
        v-model="draft"
        data-testid="issue-chat-input"
        rows="2"
        placeholder="Investigate this issue — read logs, run curl, ask questions…"
        :disabled="sending"
      ></textarea>
      <button
        class="p-button--positive is-dense"
        data-testid="issue-chat-send-btn"
        type="button"
        :disabled="sending || !draft.trim()"
        @click="send"
      >{{ sending ? 'Thinking…' : 'Send' }}</button>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onUnmounted } from 'vue';
import ChatMessage from '../chat/ChatMessage.vue';
import DiffView from './DiffView.vue';
import { sendIssueChatMessage, openProjectEvents } from '../../lib/api.js';
import { useIssueChatStore } from '../../stores/issue-chat.js';

const TRACE_EVENT_TYPES = new Set(['thinking', 'thinking_delta', 'text_delta', 'tool_call', 'tool_result']);

const props = defineProps({
  projectId:       { type: String, required: true },
  issueId:         { type: String, required: true },
  history:         { type: Array, default: () => [] },
  proposedFiles:   { type: Object, default: null },
  proposedSummary: { type: String, default: '' },
  beforeFiles:     { type: Object, default: () => ({}) },
});
const emit = defineEmits(['refresh', 'approve']);

const store = computed(() => useIssueChatStore(props.issueId));
const messages = computed(() => store.value.messages);

const draft = ref('');
const sending = ref(false);
const error = ref(null);
const lastTurnProposedSolution = ref(false);

let loadedFor = null;
watch(() => props.issueId, () => {
  if (loadedFor === props.issueId) return;
  loadedFor = props.issueId;
  lastTurnProposedSolution.value = false;
  store.value.loadFromHistory(props.history);
}, { immediate: true });

const showProposedDiff = computed(() => lastTurnProposedSolution.value && !!props.proposedFiles);

function handleProjectEvent(e) {
  if (e.issue_id !== props.issueId) return;
  if (!TRACE_EVENT_TYPES.has(e.type) || !sending.value) return;
  const s = store.value;
  if (e.type === 'thinking') s.addThinking(e.text);
  else if (e.type === 'thinking_delta') s.addThinkingDelta(e.text);
  else if (e.type === 'text_delta') s.addTextDelta(e.text);
  else if (e.type === 'tool_call') s.addToolCall(e.name, e.input, null);
  else if (e.type === 'tool_result') s.completeToolCall(e.name, e.preview);
}

let eventSource = null;
onMounted(() => {
  eventSource = openProjectEvents(props.projectId, null, handleProjectEvent);
});
onUnmounted(() => {
  eventSource?.close();
});

async function send() {
  const text = draft.value.trim();
  if (!text || sending.value) return;
  draft.value = '';
  sending.value = true;
  error.value = null;
  lastTurnProposedSolution.value = false;
  store.value.addUserMessage(text, null, []);
  store.value.startAssistantMessage();
  try {
    const response = await sendIssueChatMessage(props.projectId, props.issueId, text);
    store.value.finalizeAssistantMessage({
      answer: response.answer,
      sources: [],
      tool_calls_made: (response.chain ?? []).filter(c => c.type === 'tool_call').length,
    });
    lastTurnProposedSolution.value = !!response.proposed_solution;
    emit('refresh');
  } catch (e) {
    store.value.setError(e.message || 'Failed to send message');
    error.value = e.message || 'Failed to send message';
  } finally {
    sending.value = false;
  }
}
</script>

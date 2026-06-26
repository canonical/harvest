<template>
  <div class="chat-page">
    <div class="chat-with-history">
      <div class="chat-content">
        <div v-if="remoteLocked" class="project-lock-banner">
          {{ lockedBy }} is generating a response…
        </div>

        <div v-if="presenceOthers.length" class="project-presence-bar">
          <div
            v-for="u in presenceOthers"
            :key="u.user_id"
            class="presence-avatar"
            :title="u.name"
          >{{ initials(u.name) }}</div>
        </div>

        <div ref="messagesEl" class="messages">
          <div v-if="!projectId && !chat.messages.length" class="no-project-state chat-empty-state">
            <p>Select a project to start chatting, or use the global query below.</p>
          </div>

          <div v-else-if="projectId && !chat.messages.length" class="chat-empty-state">
            <p>Ask anything about codebases or control agents</p>
          </div>

          <ChatMessage
            v-for="(msg, i) in chat.messages"
            :key="i"
            :msg="msg"
            :is-last="i === chat.messages.length - 1"
            :repo-url-map="repoUrlMap"
            @choice="sendWithChoice"
          />
        </div>

        <div class="input-area">
          <div class="input-area__main">
            <div v-if="pendingAttachments.length" class="attachment-tray">
              <div v-for="a in pendingAttachments" :key="a.id" class="attach-chip">
                <img v-if="a.preview_url" class="attach-chip__thumb" :src="a.preview_url" alt="" />
                <span class="attach-chip__name">{{ a.name }}</span>
                <button class="attach-chip__remove" type="button" @click="chat.removePendingAttachment(a.id)">×</button>
              </div>
            </div>
            <div class="p-search-box">
              <textarea
                ref="inputEl"
                v-model="query"
                class="p-search-box__input"
                placeholder="Ask anything…"
                rows="1"
                :disabled="inputDisabled"
                @keydown.enter.exact.prevent="sendQuery"
              />
              <input
                ref="fileInput"
                type="file"
                accept="image/png,image/jpeg,image/gif,image/webp,application/pdf"
                hidden
                aria-hidden="true"
                multiple
                @change="onFileSelect"
              />
              <button
                type="button"
                class="p-search-box__button attach-btn"
                :disabled="inputDisabled"
                aria-label="Attach file"
                title="Attach image or PDF (or paste with Ctrl+V)"
                @click="fileInput?.click()"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg>
              </button>
              <button
                type="submit"
                class="p-search-box__button send-btn"
                :disabled="inputDisabled || !query.trim() && !pendingAttachments.length"
                @click="sendQuery"
              >
                <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="5" y1="12" x2="19" y2="12"/><polyline points="13 6 19 12 13 18"/></svg>
              </button>
            </div>
          </div>
          <button
            v-if="projectId"
            class="chat-history-toggle-btn"
            type="button"
            aria-label="Toggle conversation history"
            @click="historyOpen = !historyOpen"
          >
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="18" height="18" aria-hidden="true"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
          </button>
        </div>
      </div>

      <div v-if="projectId" class="chat-history-panel" :class="{ 'is-open': historyOpen }">
        <div class="chat-history-panel__header">
          <button class="chat-new-btn" data-testid="new-chat" type="button" @click="newChat">+ New chat</button>
          <button class="chat-history-close-btn" type="button" aria-label="Close history" @click="historyOpen = false">✕</button>
        </div>
        <div class="chat-history-panel__list">
          <div v-for="group in convGroups" :key="group.label" class="conv-group">
            <div class="conv-group-label">{{ group.label }}</div>
            <div
              v-for="conv in group.items"
              :key="conv.id"
              class="conv-item"
              :class="{ 'conv-item--active': conv.id === activeConvId }"
              role="button"
              tabindex="0"
              @click="loadConversation(conv.id)"
              @keydown.enter.prevent="loadConversation(conv.id)"
            >
              <span class="conv-item__title">{{ conv.title }}</span>
              <button
                v-if="canDeleteConv(conv)"
                class="conv-item__delete"
                title="Delete"
                @click.stop="confirmDeleteConversation(conv)"
              >✕</button>
            </div>
          </div>
          <p v-if="!conversations.length" class="conv-empty">No past conversations yet.</p>
        </div>
      </div>
    </div>

    <div v-if="projectId && historyOpen" class="chat-history-backdrop" @click="historyOpen = false" />
  </div>

  <SourcePanel />

  <div v-if="deletingConv" class="modal" @click.self="deletingConv = null">
    <div class="modal-content">
      <button class="modal-close" type="button" @click="deletingConv = null">✕</button>
      <h3>Delete conversation</h3>
      <p>Delete <strong>{{ deletingConv.title || 'this conversation' }}</strong>? This cannot be undone.</p>
      <div class="modal-actions">
        <button class="p-button--base" type="button" @click="deletingConv = null">Cancel</button>
        <button class="p-button--negative" type="button" @click="submitDeleteConversation">Delete</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue';
import { useAuthStore }    from '../stores/auth.js';
import { useChatStore }    from '../stores/chat.js';
import ChatMessage         from '../components/chat/ChatMessage.vue';
import SourcePanel         from '../components/SourcePanel.vue';
import { describeToolCall } from '../lib/tool-render.js';
import {
  queryStream,
  openProjectEvents,
  projectQueryStart,
  listProjectConversations,
  createProjectConversation,
  getProjectConversation,
  deleteProjectConversation,
  listConversations,
  createConversation,
  getConversation,
  deleteConversation as apiDeleteConversation,
  fetchRepositories,
} from '../lib/api.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const auth  = useAuthStore();
const chat  = useChatStore();

const query          = ref('');
const historyOpen    = ref(false);
const conversations  = ref([]);
const activeConvId   = ref(null);
const deletingConv   = ref(null);
const repoUrlMap     = ref({});
const fileInput      = ref(null);
const inputEl        = ref(null);
const messagesEl     = ref(null);

const remoteLocked   = ref(false);
const lockedBy       = ref('');
const presence       = ref([]);
let   projectEs      = null;

const pendingAttachments = computed(() => chat.pendingAttachments);

const inputDisabled = computed(() =>
  chat.loading || remoteLocked.value
);

const presenceOthers = computed(() => {
  const me = auth.user;
  return presence.value.filter(u =>
    u.user_id !== me?.id && u.conv_id === activeConvId.value
  );
});

function dayStart(d) {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

const convGroups = computed(() => {
  const now     = Date.now();
  const today   = dayStart(new Date(now));
  const yest    = today - 86_400_000;
  const week    = today - 7 * 86_400_000;
  const groups  = [
    { label: 'Today',       items: [] },
    { label: 'Yesterday',   items: [] },
    { label: 'Last 7 days', items: [] },
    { label: 'Older',       items: [] },
  ];
  for (const c of conversations.value) {
    const ms = dayStart(new Date(c.updated_at));
    if      (ms >= today) groups[0].items.push(c);
    else if (ms >= yest)  groups[1].items.push(c);
    else if (ms >= week)  groups[2].items.push(c);
    else                  groups[3].items.push(c);
  }
  return groups.filter(g => g.items.length > 0);
});

function canDeleteConv(conv) {
  if (!props.projectId) return true;
  return auth.isAdmin || conv.created_by === auth.user?.id;
}

function openEventStream() {
  closeEventStream();
  if (!props.projectId) return;
  projectEs = openProjectEvents(props.projectId, activeConvId.value, handleProjectEvent);
}

function closeEventStream() {
  projectEs?.close();
  projectEs      = null;
  remoteLocked.value = false;
  lockedBy.value     = '';
  presence.value     = [];
}

function handleProjectEvent(event) {
  switch (event.type) {
    case 'presence':
      presence.value = event.users ?? [];
      break;
    case 'user_join':
      presence.value = [
        ...presence.value.filter(u => u.user_id !== event.user_id),
        { user_id: event.user_id, name: event.name, conv_id: event.conv_id ?? null },
      ];
      break;
    case 'user_leave':
      presence.value = presence.value.filter(u => u.user_id !== event.user_id);
      break;
    case 'lock':
      if (event.conv_id === activeConvId.value) {
        remoteLocked.value = true;
        lockedBy.value     = event.by ?? '';
      }
      break;
    case 'unlock':
      if (event.conv_id === activeConvId.value) {
        remoteLocked.value = false;
        lockedBy.value     = '';
      }
      break;
    case 'user_message':
      chat.addUserMessage(event.query ?? '', event.username ?? null, event.attachments ?? []);
      chat.startAssistantMessage();
      break;
    case 'thinking':        chat.addThinking(event.text); break;
    case 'thinking_delta':  chat.addThinkingDelta(event.text); break;
    case 'text_delta':      chat.addTextDelta(event.text); break;
    case 'tool_call':
      chat.addToolCall(event.name, event.input, event.description ?? describeToolCall(event.name, event.input ?? {}, { hostname: event.hostname }));
      break;
    case 'tool_result':
      chat.completeToolCall(event.name, event.preview);
      break;
    case 'question':
      chat.setQuestion(event.question, event.choices);
      break;
    case 'done':
      chat.finalizeAssistantMessage({ answer: event.answer, sources: event.sources, tool_calls_made: event.tool_calls_made });
      updateConvTitle();
      scrollToLastMessage();
      return;
    case 'suggestions':
      chat.setSuggestions(event.choices ?? []);
      break;
    case 'error':
      chat.setError(event.message);
      break;
  }
  scrollToBottom();
}

async function loadConversationList() {
  try {
    conversations.value = props.projectId
      ? await listProjectConversations(props.projectId)
      : await listConversations();
  } catch {
    conversations.value = [];
  }
}

async function loadConversation(id) {
  try {
    const conv = props.projectId
      ? await getProjectConversation(props.projectId, id)
      : await getConversation(id);
    activeConvId.value = id;
    chat.loadFromHistory(Array.isArray(conv.messages) ? conv.messages : []);
    historyOpen.value = false;
    openEventStream();
    await nextTick();
    scrollToBottom();
  } catch {}
}

function confirmDeleteConversation(conv) {
  deletingConv.value = conv;
}

async function submitDeleteConversation() {
  const conv = deletingConv.value;
  if (!conv) return;
  try {
    if (props.projectId) {
      await deleteProjectConversation(props.projectId, conv.id);
    } else {
      await apiDeleteConversation(conv.id);
    }
    conversations.value = conversations.value.filter(c => c.id !== conv.id);
    if (activeConvId.value === conv.id) newChat();
    deletingConv.value = null;
  } catch {}
}

function newChat() {
  activeConvId.value = null;
  chat.reset();
  openEventStream();
  loadConversationList();
}

function updateConvTitle() {
  if (!activeConvId.value) return;
  const idx = conversations.value.findIndex(c => c.id === activeConvId.value);
  if (idx !== -1) {
    conversations.value[idx] = { ...conversations.value[idx], updated_at: new Date().toISOString() };
    conversations.value.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  }
}

function initials(name) {
  return (name ?? '').split(/\s+/).map(w => w[0]).join('').toUpperCase().slice(0, 2);
}

function toWireAttachment(a) {
  return { name: a.name, mime_type: a.mime_type, data: a.data };
}

async function sendQuery() {
  const text = query.value.trim();
  if ((!text && !pendingAttachments.value.length) || chat.loading || remoteLocked.value) return;
  query.value = '';

  const attachments = pendingAttachments.value.map(toWireAttachment);
  chat.clearPendingAttachments();

  if (props.projectId) {
    try {
      if (!activeConvId.value) {
        const title = text.length > 60 ? text.slice(0, 57) + '…' : text;
        const conv = await createProjectConversation(props.projectId, { title });
        activeConvId.value = conv.id;
        conversations.value = [conv, ...conversations.value];
        openEventStream();
      }
      await projectQueryStart(props.projectId, text, activeConvId.value, attachments);
    } catch (err) {
      if (err.status !== 409) {
        chat.addUserMessage(text, auth.user?.name ?? null, attachments);
        chat.startAssistantMessage();
        chat.setError(err.message);
      }
    }
    return;
  }

  chat.addUserMessage(text, null, attachments);
  chat.startAssistantMessage();
  await nextTick(); scrollToBottom();

  try {
    if (!activeConvId.value) {
      const title = text.length > 60 ? text.slice(0, 57) + '…' : text;
      const conv = await createConversation(title);
      activeConvId.value = conv.id;
      conversations.value = [conv, ...conversations.value];
    }
    await queryStream(text, activeConvId.value, attachments, (event) => {
      handleStreamEvent(event);
      if (event.type === 'done') scrollToLastMessage();
      else scrollToBottom();
    });
  } catch (err) {
    chat.setError(err.message);
  }
}

function sendWithChoice(text) {
  query.value = text;
  sendQuery();
}

function handleStreamEvent(event) {
  switch (event.type) {
    case 'thinking':        chat.addThinking(event.text); break;
    case 'thinking_delta':  chat.addThinkingDelta(event.text); break;
    case 'text_delta':      chat.addTextDelta(event.text); break;
    case 'tool_call':  chat.addToolCall(event.name, event.input, event.description ?? describeToolCall(event.name, event.input ?? {}, { hostname: event.hostname })); break;
    case 'tool_result': chat.completeToolCall(event.name, event.preview); break;
    case 'question':   chat.setQuestion(event.question, event.choices); break;
    case 'done':
      chat.finalizeAssistantMessage({ answer: event.answer, sources: event.sources, tool_calls_made: event.tool_calls_made });
      updateConvTitle();
      break;
    case 'suggestions': chat.setSuggestions(event.choices ?? []); break;
    case 'error':      chat.setError(event.message); break;
  }
}

async function addFile(file) {
  const ACCEPTED = ['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'application/pdf'];
  if (!ACCEPTED.includes(file.type)) return;
  const data = await new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload  = () => resolve(reader.result.split(',')[1]);
    reader.onerror = reject;
    reader.readAsDataURL(file);
  });
  const preview_url = file.type.startsWith('image/') ? `data:${file.type};base64,${data}` : null;
  chat.addPendingAttachment({ name: file.name, mime_type: file.type, data, preview_url });
}

function onFileSelect(e) {
  [...(e.target.files ?? [])].forEach(addFile);
  e.target.value = '';
}

function scrollToBottom() {
  nextTick(() => {
    if (messagesEl.value) {
      messagesEl.value.scrollTop = messagesEl.value.scrollHeight;
    }
  });
}

// After a response completes, scroll so the START of the latest assistant message
// is at the top of the viewport.  This keeps preambles + tool calls visible above
// the final answer rather than scrolling them off-screen.
function scrollToLastMessage() {
  nextTick(() => {
    const container = messagesEl.value;
    if (!container) return;
    const msgs = container.querySelectorAll('.message--assistant');
    if (!msgs.length) { scrollToBottom(); return; }
    const last = msgs[msgs.length - 1];
    const containerTop = container.getBoundingClientRect().top;
    const lastTop      = last.getBoundingClientRect().top;
    container.scrollTop += lastTop - containerTop - 8;
  });
}

function autoResizeInput() {
  const el = inputEl.value;
  if (!el) return;
  el.style.height = 'auto';
  el.style.height = `${el.scrollHeight}px`;
}

watch(query, () => nextTick(autoResizeInput));

onMounted(async () => {
  chat.reset();
  if (props.projectId) {
    openEventStream();
    await loadConversationList();
  } else {
    await loadConversationList();
  }
  try {
    const repos = await fetchRepositories();
    repoUrlMap.value = Object.fromEntries(
      repos.filter(r => r.url).map(r => [r.name, r.url])
    );
  } catch {}

  document.addEventListener('paste', onPaste);
});

onUnmounted(() => {
  closeEventStream();
  document.removeEventListener('paste', onPaste);
});

watch(() => props.projectId, async (newId) => {
  chat.reset();
  activeConvId.value = null;
  closeEventStream();
  if (newId) {
    openEventStream();
    await loadConversationList();
  } else {
    conversations.value = [];
  }
});

function onPaste(e) {
  const items = [...(e.clipboardData?.items ?? [])];
  for (const item of items) {
    if (item.kind === 'file') {
      const f = item.getAsFile();
      if (f) addFile(f);
    }
  }
}
</script>

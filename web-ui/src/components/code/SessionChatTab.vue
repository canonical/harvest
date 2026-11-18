<template>
  <div class="session-tab">
    <div v-if="chat" class="session-tab__body">
      <ChatPaneContent
        :key="instanceKey"
        :tab-id="instanceKey"
        :conversation-id="chat.id"
        :code-session-id="workspace.sessionId"
        :unavailable-reason="workspace.chatUnavailableReason"
        @title-updated="renameLocally"
      />
    </div>
    <div v-else class="session-tab__empty">
      <p>This session has no chats yet.</p>
      <button class="p-button--positive is-dense" type="button" @click="startChat">Start a chat</button>
    </div>

    <AppModal v-if="renaming" title="Rename chat" @close="renaming = false">
      <form class="app-form" @submit.prevent="saveTitle">
      <label :for="`rename-${tab.tabId}`">Title</label>
      <input :id="`rename-${tab.tabId}`" v-model="title" type="text" maxlength="120" required />
      </form>
      <template #actions>
      <button class="p-button--base is-dense" type="button" @click="renaming = false">Cancel</button>
      <button class="p-button--positive is-dense" type="button" :disabled="!title.trim()" @click="saveTitle">Rename</button>
      </template>
    </AppModal>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import ChatPaneContent from '../chat/ChatPaneContent.vue';
import AppModal from '../ui/AppModal.vue';
import { useLayoutWorkspace } from '../../lib/layout-workspace.js';
import { chatInstanceKey } from '../../composables/useSessionWorkspace.js';
import { disposeChatInstance, peekChatInstance, repliesIn } from '../../composables/useChatInstance.js';
import { showToast } from '../../composables/useToasts.js';

const props = defineProps({
  tab: { type: Object, required: true },
  paneId: { type: String, required: true },
});

const workspace = useLayoutWorkspace();
const renaming = ref(false);
const title = ref('');

const chat = computed(() => workspace.chats.find(c => c.id === props.tab.conversationId) ?? null);
const instanceKey = computed(() => chatInstanceKey(props.tab));
const replies = computed(() => repliesIn(peekChatInstance(instanceKey.value)));

function show(chatId) {
  if (chatId === props.tab.conversationId) return;
  disposeChatInstance(instanceKey.value);
  workspace.updateTab(props.tab.tabId, { conversationId: chatId, seenReplies: 0 });
}

watch(
  () => [props.tab.conversationId, workspace.chats.length],
  () => {
    if (!chat.value && workspace.chats.length) show(workspace.chats[0].id);
  },
  { immediate: true },
);

watch(replies, (count) => {
  if (count !== props.tab.seenReplies) workspace.updateTab(props.tab.tabId, { seenReplies: count });
}, { immediate: true });

watch(() => workspace.tabRequest, (request) => {
  if (request?.tabId !== props.tab.tabId || !chat.value) return;
  if (request.action === 'rename-chat') {
    title.value = chat.value.title;
    renaming.value = true;
  } else if (request.action === 'delete-chat') {
    workspace.deleteChat(chat.value.id);
  }
});

async function startChat() {
  try {
    show((await workspace.newChat()).id);
  } catch (e) {
    showToast({ message: e.message, kind: 'negative' });
  }
}

async function saveTitle() {
  const text = title.value.trim();
  if (!text) return;
  try {
    await workspace.renameChat(chat.value.id, text);
    renaming.value = false;
  } catch (e) {
    showToast({ message: e.message, kind: 'negative' });
  }
}

function renameLocally(newTitle) {
  const entry = workspace.chats.find(c => c.id === props.tab.conversationId);
  if (entry) entry.title = newTitle;
}
</script>

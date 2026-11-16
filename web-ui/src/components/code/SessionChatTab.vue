<template>
  <div class="session-tab">
    <div class="session-tab__bar">
      <select v-if="workspace.chats.length" class="session-tab__picker" :value="chat?.id" aria-label="Chat" @change="switchTo($event.target.value)">
        <option v-for="entry in workspace.chats" :key="entry.id" :value="entry.id">{{ entry.title }}</option>
      </select>
      <button class="p-button--base is-dense" type="button" :disabled="busy" @click="startChat">New chat</button>
      <button v-if="chat" class="p-button--base is-dense" type="button" :disabled="busy" @click="removeChat">Delete chat</button>
    </div>
    <p v-if="error" class="code-error" role="alert">{{ error }}</p>
    <div v-if="chat" class="session-tab__body">
      <ChatPaneContent
        :key="instanceKey"
        :tab-id="instanceKey"
        :conversation-id="chat.id"
        :code-session-id="workspace.sessionId"
        @title-updated="renameLocally"
      />
    </div>
    <p v-else class="session-tab__empty u-text--muted">This session has no chats. Start one to give the agent a task.</p>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import ChatPaneContent from '../chat/ChatPaneContent.vue';
import { useLayoutWorkspace } from '../../lib/layout-workspace.js';
import { chatInstanceKey } from '../../composables/useSessionWorkspace.js';
import { disposeChatInstance } from '../../composables/useChatInstance.js';

const props = defineProps({ tab: { type: Object, required: true } });

const workspace = useLayoutWorkspace();
const busy = ref(false);
const error = ref('');

const chat = computed(() => workspace.chats.find(c => c.id === props.tab.conversationId) ?? null);
const instanceKey = computed(() => chatInstanceKey(props.tab));

function switchTo(chatId) {
  if (chatId === props.tab.conversationId) return;
  disposeChatInstance(instanceKey.value);
  workspace.updateTab(props.tab.tabId, { conversationId: chatId });
}

watch(
  () => [props.tab.conversationId, workspace.chats.length],
  () => {
    if (!chat.value && workspace.chats.length) switchTo(workspace.chats[0].id);
  },
  { immediate: true },
);

async function run(action) {
  busy.value = true;
  error.value = '';
  try {
    await action();
  } catch (e) {
    error.value = e.message;
  } finally {
    busy.value = false;
  }
}

function startChat() {
  return run(async () => switchTo((await workspace.newChat()).id));
}

function removeChat() {
  if (!window.confirm(`Delete the chat “${chat.value.title}”? Its messages cannot be recovered.`)) return;
  return run(async () => {
    const deleted = chat.value.id;
    disposeChatInstance(instanceKey.value);
    await workspace.deleteChat(deleted);
  });
}

function renameLocally(title) {
  const entry = workspace.chats.find(c => c.id === props.tab.conversationId);
  if (entry) entry.title = title;
}
</script>

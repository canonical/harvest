<template>
  <ChatPaneContent
    :tab-id="tab.tabId"
    :conversation-id="tab.conversationId"
    :project-id="tab.projectId"
    @update:conversation-id="(conversationId) => workspace.updateTab(tab.tabId, { conversationId })"
    @title-updated="(title) => workspace.updateTab(tab.tabId, { title })"
  />
</template>

<script setup>
import { computed, watch } from 'vue';
import ChatPaneContent from './ChatPaneContent.vue';
import { useLayoutWorkspace } from '../../lib/layout-workspace.js';
import { peekChatInstance, repliesIn } from '../../composables/useChatInstance.js';

const props = defineProps({
  tab: { type: Object, required: true },
  paneId: { type: String, required: true },
});

const workspace = useLayoutWorkspace();
const replies = computed(() => repliesIn(peekChatInstance(props.tab.tabId)));

watch(replies, (count) => {
  if (count !== props.tab.seenReplies) workspace.updateTab(props.tab.tabId, { seenReplies: count });
}, { immediate: true });
</script>

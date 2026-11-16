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
import ChatPaneContent from './ChatPaneContent.vue';
import { useLayoutWorkspace } from '../../lib/layout-workspace.js';

defineProps({
  tab: { type: Object, required: true },
  paneId: { type: String, required: true },
});

const workspace = useLayoutWorkspace();
</script>

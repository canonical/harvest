<template>
  <component :is="KINDS[tab.kind] ?? SessionChatTab" :tab="tab" :pane-id="paneId" />
</template>

<script setup>
import SessionChatTab from './SessionChatTab.vue';
import SessionChanges from './SessionChanges.vue';
import SessionPreviewTab from './SessionPreviewTab.vue';
import SessionJobsTab from './SessionJobsTab.vue';
import SessionTerminalTab from './SessionTerminalTab.vue';

defineProps({
  tab: { type: Object, required: true },
  paneId: { type: String, required: true },
});

const KINDS = {
  chat: SessionChatTab,
  changes: SessionChanges,
  preview: SessionPreviewTab,
  jobs: SessionJobsTab,
  terminal: SessionTerminalTab,
};
</script>

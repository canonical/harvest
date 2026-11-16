<template>
  <component :is="KINDS[tab.kind] ?? SessionChatTab" :tab="tab" />
</template>

<script setup>
import SessionChatTab from './SessionChatTab.vue';
import SessionChanges from './SessionChanges.vue';
import SessionPreviewTab from './SessionPreviewTab.vue';

defineProps({
  tab: { type: Object, required: true },
  paneId: { type: String, required: true },
});

const KINDS = { chat: SessionChatTab, changes: SessionChanges, preview: SessionPreviewTab };
</script>

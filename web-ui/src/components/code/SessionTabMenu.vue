<template>
  <ContextualMenu class="session-tab-menu" :items="items" :label="label" align="left" @select="choose" @close="emit('close')">
    <template v-if="tab.kind === 'chat'" #header>
      <input v-model="filter" class="app-menu__filter" type="search" placeholder="Filter chats" aria-label="Filter chats" />
    </template>
  </ContextualMenu>
</template>

<script setup>
import { computed, ref } from 'vue';
import ContextualMenu from '../ui/ContextualMenu.vue';
import { useLayoutWorkspace } from '../../lib/layout-workspace.js';
import { chatMenuItems, previewMenuItems } from '../../lib/session-menus.js';

const props = defineProps({ tab: { type: Object, required: true } });
const emit = defineEmits(['close']);

const workspace = useLayoutWorkspace();
const filter = ref('');

const label = computed(() => (props.tab.kind === 'chat' ? 'Chats of this session' : 'Previews of this session'));
const items = computed(() => (props.tab.kind === 'chat'
  ? chatMenuItems(workspace.chats, props.tab.conversationId, filter.value)
  : previewMenuItems(workspace.previews, props.tab.previewId)));

async function choose(item) {
  emit('close');
  if (item.id === 'new-chat') {
    const chat = await workspace.newChat();
    workspace.updateTab(props.tab.tabId, { conversationId: chat.id });
  } else if (item.action) {
    workspace.requestTabAction(props.tab.tabId, item.action);
  } else if (props.tab.kind === 'chat') {
    workspace.updateTab(props.tab.tabId, { conversationId: item.id });
  } else {
    workspace.updateTab(props.tab.tabId, { previewId: item.id });
  }
}
</script>

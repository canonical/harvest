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
import { chatMenuItems, previewMenuItems, terminalMenuItems } from '../../lib/session-menus.js';

const props = defineProps({ tab: { type: Object, required: true } });
const emit = defineEmits(['close']);

const workspace = useLayoutWorkspace();
const filter = ref('');

const MENUS = {
  chat: { label: 'Chats of this session', items: () => chatMenuItems(workspace.chats, props.tab.conversationId, filter.value) },
  preview: { label: 'Previews of this session', items: () => previewMenuItems(workspace.previews, props.tab.previewId) },
  terminal: { label: 'Terminals of this session', items: () => terminalMenuItems(workspace.terminals, props.tab.terminalId) },
};

const menu = computed(() => MENUS[props.tab.kind] ?? MENUS.preview);
const label = computed(() => menu.value.label);
const items = computed(() => menu.value.items());

async function choose(item) {
  emit('close');
  if (item.id === 'new-chat') {
    const chat = await workspace.newChat();
    workspace.updateTab(props.tab.tabId, { conversationId: chat.id });
  } else if (item.id === 'new-terminal') {
    const terminal = await workspace.newTerminal();
    workspace.updateTab(props.tab.tabId, { terminalId: terminal.id, title: terminal.title });
  } else if (item.action) {
    workspace.requestTabAction(props.tab.tabId, item.action);
  } else if (props.tab.kind === 'chat') {
    workspace.updateTab(props.tab.tabId, { conversationId: item.id });
  } else if (props.tab.kind === 'terminal') {
    workspace.updateTab(props.tab.tabId, { terminalId: item.id, title: item.label });
  } else {
    workspace.updateTab(props.tab.tabId, { previewId: item.id });
  }
}
</script>

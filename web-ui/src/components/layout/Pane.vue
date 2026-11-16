<template>
  <div class="pane" :class="{ 'is-focused': isFocused }" @focusin="focusPane" @pointerdown="focusPane">
    <div class="pane__tabstrip-row">
      <div class="p-tabs pane__tabstrip" @dragover.prevent @drop="onTabstripDrop">
        <ul class="p-tabs__list" :aria-label="`Tabs of this pane`">
          <li
            v-for="tab in node.tabs"
            :key="tab.tabId"
            class="p-tabs__item pane__tab-item"
            draggable="true"
            @dragstart="onTabDragStart($event, tab.tabId)"
            @dragend="onTabDragEnd"
          >
            <button
              class="p-tabs__link pane__tab-link"
              type="button"
              :aria-current="tab.tabId === node.activeTabId ? 'page' : undefined"
              :title="titleOf(tab)"
              @click="activate(tab.tabId)"
            >
              <span v-if="activityOf(tab) === 'working'" class="pane__tab-activity pane__tab-activity--working" aria-label="Working" />
              <span v-else-if="activityOf(tab) === 'unseen'" class="pane__tab-activity pane__tab-activity--unseen" aria-label="New reply" />
              <span class="pane__tab-title">{{ titleOf(tab) }}</span>
            </button>
            <button
              v-if="workspace.tabMenu && workspace.hasTabMenu(tab) && tab.tabId === node.activeTabId"
              class="pane__tab-menu"
              type="button"
              aria-haspopup="menu"
              :aria-expanded="String(tabMenuOpen)"
              :aria-label="`Switch what ${titleOf(tab)} shows`"
              title="Switch"
              @click.stop="tabMenuOpen = !tabMenuOpen"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="4 6 8 10 12 6"/></svg>
            </button>
            <button
              class="pane__tab-close"
              type="button"
              :aria-label="`Close ${titleOf(tab)}`"
              title="Close tab (Ctrl+Alt+W)"
              @click.stop="closeTab(tab.tabId)"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
            </button>
          </li>
        </ul>
      </div>

      <component
        :is="workspace.tabMenu"
        v-if="tabMenuOpen && activeTab"
        :tab="activeTab"
        @close="tabMenuOpen = false"
      />

      <button
        class="pane__toolbar-btn p-button--base has-icon is-dense u-no-margin"
        type="button"
        aria-label="New tab"
        title="New tab (Ctrl+Alt+T)"
        :aria-expanded="String(choosingFor === 'tab')"
        @click="request('tab')"
      >
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
      </button>

      <div class="pane__toolbar">
        <button
          v-for="edge in EDGES"
          :key="edge"
          class="pane__toolbar-btn p-button--base has-icon is-dense u-no-margin"
          type="button"
          :aria-label="`Split ${edge}`"
          :title="`Split ${edge} (${SHORTCUT_LABELS[edge]})`"
          :aria-expanded="String(choosingFor === edge)"
          @click="request(edge)"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24"
            fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            aria-hidden="true"
            :class="{ 'pane__split-icon--rotated': edge === 'bottom' }"
          ><rect x="3" y="3" width="18" height="18" rx="1"/><line x1="12" y1="3" x2="12" y2="21"/></svg>
        </button>
        <button
          class="pane__toolbar-btn p-button--base has-icon is-dense u-no-margin"
          type="button"
          aria-label="Close pane"
          title="Close pane"
          @click="closePane"
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>
      </div>

      <ContextualMenu
        v-if="choosingFor"
        class="pane__kind-menu"
        :items="kindItems"
        :label="choosingFor === 'tab' ? 'Open a new tab' : `Split ${choosingFor}`"
        @select="(item) => create(item.id)"
        @close="choosingFor = null"
      >
        <template #header><span class="app-menu__heading">{{ choosingFor === 'tab' ? 'Open a new tab' : `Split ${choosingFor}` }}</span></template>
      </ContextualMenu>
    </div>

    <div class="pane__body">
      <component
        :is="workspace.content"
        v-if="activeTab"
        :key="activeTab.tabId"
        :tab="activeTab"
        :pane-id="node.id"
      />
      <div v-else class="pane__empty">No tabs open</div>

      <template v-if="isDragActive">
        <div
          v-for="edge in DROP_EDGES"
          :key="edge"
          class="pane__drop-overlay"
          :class="[`pane__drop-overlay--${edge}`, { 'pane__drop-overlay--active': hoveredEdge === edge }]"
          @dragenter.prevent="hoveredEdge = edge"
          @dragover.prevent
          @dragleave="onEdgeDragLeave(edge)"
          @drop="onEdgeDrop(edge)"
        />
      </template>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import ContextualMenu from '../ui/ContextualMenu.vue';
import { useLayoutWorkspace, usePaneFocus } from '../../lib/layout-workspace.js';
import { dragState, startTabDrag, endTabDrag } from '../../lib/pane-drag-state.js';
import { paneShortcut } from '../../lib/pane-shortcuts.js';

const props = defineProps({
  node: { type: Object, required: true },
});

const workspace = useLayoutWorkspace();
const focusedPane = usePaneFocus();

const EDGES = ['right', 'bottom'];
const DROP_EDGES = ['top', 'bottom', 'left', 'right'];
const SHORTCUT_LABELS = { tab: 'Ctrl+Alt+T', right: 'Ctrl+Alt+\\', bottom: 'Ctrl+Alt+-' };

const activeTab = computed(() => props.node.tabs.find(t => t.tabId === props.node.activeTabId) ?? null);
const isDragActive = computed(() => dragState.value.draggingTabId !== null && dragState.value.workspaceId === workspace.id);
const isFocused = computed(() => focusedPane.value === props.node.id);
const kindItems = computed(() => workspace.tabKinds.map(k => ({ id: k.kind, label: k.label, icon: k.icon })));
const hoveredEdge = ref(null);
const choosingFor = ref(null);
const tabMenuOpen = ref(false);

watch(isDragActive, (active) => { if (!active) hoveredEdge.value = null; });
watch(() => props.node.activeTabId, () => { tabMenuOpen.value = false; });

function titleOf(tab) {
  return workspace.titleOf(tab);
}

function activityOf(tab) {
  return workspace.activityOf?.(tab, tab.tabId === props.node.activeTabId) ?? null;
}

function focusPane() {
  focusedPane.value = props.node.id;
}

function onEdgeDragLeave(edge) {
  if (hoveredEdge.value === edge) hoveredEdge.value = null;
}

function activate(tabId) {
  workspace.setActiveTab(props.node.id, tabId);
}

function closeTab(tabId) {
  workspace.closeTab(props.node.id, tabId);
}

function closePane() {
  workspace.closePane(props.node.id);
}

function request(placement) {
  if (workspace.tabKinds.length === 1) {
    create(workspace.tabKinds[0].kind, placement);
    return;
  }
  choosingFor.value = choosingFor.value === placement ? null : placement;
}

async function create(kind, placement = choosingFor.value) {
  choosingFor.value = null;
  const tab = await workspace.createTab(kind, { paneId: props.node.id, activeTab: activeTab.value });
  if (!tab) return;
  if (placement === 'tab') workspace.openTab(props.node.id, tab);
  else workspace.splitPaneWithTab(props.node.id, placement, tab);
}

function onShortcut(event) {
  if (!isFocused.value) return;
  const action = paneShortcut(event);
  if (!action) return;
  event.preventDefault();
  if (action === 'close') {
    if (activeTab.value) closeTab(activeTab.value.tabId);
  } else {
    request(action);
  }
}

function onTabDragStart(e, tabId) {
  e.dataTransfer.setData('text/plain', tabId);
  e.dataTransfer.effectAllowed = 'move';
  startTabDrag(tabId, props.node.id, workspace.id);
}

function onTabDragEnd() {
  endTabDrag();
}

function onTabstripDrop() {
  const { draggingTabId, sourcePaneId } = dragState.value;
  if (draggingTabId && isDragActive.value) workspace.moveTab(sourcePaneId, draggingTabId, props.node.id, -1);
  endTabDrag();
}

function onEdgeDrop(edge) {
  const { draggingTabId, sourcePaneId } = dragState.value;
  if (draggingTabId && isDragActive.value) workspace.moveTabToNewSplit(sourcePaneId, draggingTabId, props.node.id, edge);
  hoveredEdge.value = null;
  endTabDrag();
}

onMounted(() => {
  focusedPane.value ??= props.node.id;
  window.addEventListener('keydown', onShortcut);
});

onUnmounted(() => {
  window.removeEventListener('keydown', onShortcut);
  if (focusedPane.value === props.node.id) focusedPane.value = null;
});
</script>

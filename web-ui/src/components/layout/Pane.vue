<template>
  <div class="pane">
    <div class="pane__tabstrip-row">
      <div class="p-tabs pane__tabstrip" @dragover.prevent @drop="onTabstripDrop">
        <ul class="p-tabs__list" role="tablist">
          <li
            v-for="tab in node.tabs"
            :key="tab.tabId"
            class="p-tabs__item pane__tab-item"
            role="presentation"
            draggable="true"
            @dragstart="onTabDragStart($event, tab.tabId)"
            @dragend="onTabDragEnd"
          >
            <button
              class="p-tabs__link pane__tab-link"
              role="tab"
              type="button"
              :aria-selected="tab.tabId === node.activeTabId"
              :title="titleOf(tab)"
              @click="activate(tab.tabId)"
            ><span class="pane__tab-title">{{ titleOf(tab) }}</span></button>
            <button
              class="pane__tab-close"
              type="button"
              aria-label="Close tab"
              title="Close tab"
              @click.stop="closeTab(tab.tabId)"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
            </button>
          </li>
        </ul>
      </div>

      <button
        class="pane__toolbar-btn p-button--base has-icon is-dense u-no-margin"
        type="button"
        aria-label="New tab"
        title="New tab"
        :aria-expanded="choosingFor === 'tab'"
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
          :title="`Split ${edge}`"
          :aria-expanded="choosingFor === edge"
          @click="request(edge)"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24"
            fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            aria-hidden="true"
            :class="{ 'pane__split-icon--rotated': edge === 'top' || edge === 'bottom' }"
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

      <TabKindMenu
        v-if="choosingFor"
        :kinds="workspace.tabKinds"
        :label="choosingFor === 'tab' ? 'Open a new tab' : `Split ${choosingFor}`"
        @choose="create"
        @close="choosingFor = null"
      />
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
          class="pane__drop-overlay pane__drop-overlay--top"
          :class="{ 'pane__drop-overlay--active': hoveredEdge === 'top' }"
          @dragenter.prevent="hoveredEdge = 'top'"
          @dragover.prevent
          @dragleave="onEdgeDragLeave('top')"
          @drop="onEdgeDrop('top', $event)"
        />
        <div
          class="pane__drop-overlay pane__drop-overlay--bottom"
          :class="{ 'pane__drop-overlay--active': hoveredEdge === 'bottom' }"
          @dragenter.prevent="hoveredEdge = 'bottom'"
          @dragover.prevent
          @dragleave="onEdgeDragLeave('bottom')"
          @drop="onEdgeDrop('bottom', $event)"
        />
        <div
          class="pane__drop-overlay pane__drop-overlay--left"
          :class="{ 'pane__drop-overlay--active': hoveredEdge === 'left' }"
          @dragenter.prevent="hoveredEdge = 'left'"
          @dragover.prevent
          @dragleave="onEdgeDragLeave('left')"
          @drop="onEdgeDrop('left', $event)"
        />
        <div
          class="pane__drop-overlay pane__drop-overlay--right"
          :class="{ 'pane__drop-overlay--active': hoveredEdge === 'right' }"
          @dragenter.prevent="hoveredEdge = 'right'"
          @dragover.prevent
          @dragleave="onEdgeDragLeave('right')"
          @drop="onEdgeDrop('right', $event)"
        />
      </template>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import TabKindMenu from './TabKindMenu.vue';
import { useLayoutWorkspace } from '../../lib/layout-workspace.js';
import { dragState, startTabDrag, endTabDrag } from '../../lib/pane-drag-state.js';

const props = defineProps({
  node: { type: Object, required: true },
});

const workspace = useLayoutWorkspace();

const EDGES = ['right', 'bottom'];

const activeTab = computed(() => props.node.tabs.find(t => t.tabId === props.node.activeTabId) ?? null);
const isDragActive = computed(() => dragState.value.draggingTabId !== null && dragState.value.workspaceId === workspace.id);
const hoveredEdge = ref(null);
const choosingFor = ref(null);

watch(isDragActive, (active) => { if (!active) hoveredEdge.value = null; });

function titleOf(tab) {
  return workspace.titleOf(tab);
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
</script>

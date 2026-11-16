import { ref } from 'vue';

export const dragState = ref({ draggingTabId: null, sourcePaneId: null, workspaceId: null });

export function startTabDrag(tabId, paneId, workspaceId) {
  dragState.value = { draggingTabId: tabId, sourcePaneId: paneId, workspaceId };
}

export function endTabDrag() {
  dragState.value = { draggingTabId: null, sourcePaneId: null, workspaceId: null };
}

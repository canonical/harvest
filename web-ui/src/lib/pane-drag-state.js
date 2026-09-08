import { ref } from 'vue';

export const dragState = ref({ draggingTabId: null, sourcePaneId: null });

export function startTabDrag(tabId, paneId) {
  dragState.value = { draggingTabId: tabId, sourcePaneId: paneId };
}

export function endTabDrag() {
  dragState.value = { draggingTabId: null, sourcePaneId: null };
}

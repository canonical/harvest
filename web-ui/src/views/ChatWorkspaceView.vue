<template>
  <div class="chat-workspace">
    <div class="chat-workspace__toolbar">
      <div class="chat-workspace__toolbar-actions">
        <button
          class="p-button--base is-dense u-no-margin"
          type="button"
          data-testid="new-layout-btn"
          @click="onNewLayout"
        >New layout</button>
      </div>
      <div class="chat-workspace__layouts">
        <button
          class="p-button--base is-dense u-no-margin chat-workspace__layouts-toggle"
          type="button"
          :aria-expanded="layoutsOpen"
          @click="toggleLayouts"
        >
          Layouts
          <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="4 6 8 10 12 6"/></svg>
        </button>

        <div v-if="layoutsOpen" class="chat-workspace__layouts-dropdown">
          <div class="chat-workspace__layouts-list">
            <div
              v-for="layout in workspace.namedLayouts"
              :key="layout.id"
              class="layout-item"
              :class="{ 'layout-item--active': layout.id === workspace.currentNamedLayoutId }"
              role="button"
              tabindex="0"
              @click="applyNamedLayout(layout.id)"
              @keydown.enter.prevent="applyNamedLayout(layout.id)"
            >
              <LayoutShapeIcon class="layout-item__shape" :tree="layout.tree" />
              <span class="layout-item__title">{{ layout.name }}</span>
              <button class="layout-item__delete" title="Delete" @click.stop="removeNamedLayout(layout.id)">✕</button>
            </div>
            <p v-if="!workspace.namedLayouts.length" class="conv-empty">No saved layouts yet.</p>
          </div>
          <div class="chat-workspace__layouts-save">
            <input
              v-model="newLayoutName"
              type="text"
              placeholder="Save current arrangement as…"
              @keydown.enter.prevent="submitSaveLayout"
            />
            <button
              class="p-button--positive is-dense u-no-margin chat-workspace__layouts-save-btn"
              type="button"
              :disabled="!newLayoutName.trim()"
              @click="submitSaveLayout"
            >Save</button>
          </div>
        </div>
      </div>
    </div>

    <div class="chat-workspace__body">
      <PaneSplit :node="workspace.tree.root" />
    </div>
  </div>

  <SourcePanel />

  <div
    v-if="newLayoutConfirm"
    class="modal"
    role="dialog"
    aria-modal="true"
    aria-labelledby="new-layout-confirm-title"
    @click.self="newLayoutConfirm = false"
  >
    <div class="modal-content" data-testid="new-layout-confirm">
      <button class="modal-close" type="button" aria-label="Close" @click="newLayoutConfirm = false">✕</button>
      <h3 id="new-layout-confirm-title">Save current layout?</h3>
      <p>The current layout has unsaved changes. Do you want to save it before creating a new one?</p>
      <div class="modal-actions">
        <button class="p-button--base is-dense" type="button" @click="newLayoutConfirm = false">Cancel</button>
        <button class="p-button--base is-dense" type="button" data-testid="new-layout-dont-save-btn" @click="confirmNewLayoutWithoutSave">Don't save</button>
        <button
          class="p-button--positive is-dense"
          type="button"
          data-testid="new-layout-save-btn"
          :disabled="!newLayoutSaveName.trim()"
          @click="confirmNewLayoutWithSave"
        >Save and continue</button>
      </div>
      <div class="form-group" style="margin-top: 0.75rem;">
        <input
          v-model="newLayoutSaveName"
          type="text"
          class="new-layout-save-name-input"
          :placeholder="currentLayoutNamePlaceholder"
          @keydown.enter.prevent="confirmNewLayoutWithSave"
        />
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import PaneSplit from '../components/chat/PaneSplit.vue';
import LayoutShapeIcon from '../components/chat/LayoutShapeIcon.vue';
import SourcePanel from '../components/SourcePanel.vue';
import { useChatWorkspaceStore } from '../stores/chatWorkspace.js';

defineProps({ projectId: { type: String, default: null } });

const workspace = useChatWorkspaceStore();

const layoutsOpen   = ref(false);
const newLayoutName = ref('');
const newLayoutConfirm   = ref(false);
const newLayoutSaveName  = ref('');

const currentLayoutNamePlaceholder = computed(() => {
  const id = workspace.currentNamedLayoutId;
  if (id) {
    const found = workspace.namedLayouts.find(l => l.id === id);
    if (found) return found.name;
  }
  return 'Layout name…';
});

async function toggleLayouts() {
  layoutsOpen.value = !layoutsOpen.value;
  if (layoutsOpen.value) await workspace.listNamedLayouts();
}

async function applyNamedLayout(id) {
  await workspace.loadNamedLayout(id);
  layoutsOpen.value = false;
}

async function removeNamedLayout(id) {
  await workspace.deleteNamedLayout(id);
}

async function submitSaveLayout() {
  const name = newLayoutName.value.trim();
  if (!name) return;
  workspace.currentNamedLayoutId = null;
  await workspace.saveNamedLayout(name);
  newLayoutName.value = '';
}

function onNewLayout() {
  if (workspace.isLayoutSaved) {
    workspace.newLayout();
    layoutsOpen.value = false;
  } else {
    newLayoutSaveName.value = currentLayoutNamePlaceholder.value === 'Layout name…' ? '' : currentLayoutNamePlaceholder.value;
    newLayoutConfirm.value = true;
  }
}

async function confirmNewLayoutWithSave() {
  const name = newLayoutSaveName.value.trim();
  if (!name) return;
  await workspace.saveNamedLayout(name);
  workspace.newLayout();
  newLayoutConfirm.value = false;
  newLayoutSaveName.value = '';
  layoutsOpen.value = false;
}

function confirmNewLayoutWithoutSave() {
  workspace.newLayout();
  newLayoutConfirm.value = false;
  layoutsOpen.value = false;
}

function handleUnload() {
  workspace.flushAutosaveNow();
}

function handleVisibilityChange() {
  if (document.visibilityState === 'hidden') workspace.flushAutosaveNow();
}

onMounted(async () => {
  await workspace.initFromServer();
  window.addEventListener('beforeunload', handleUnload);
  document.addEventListener('visibilitychange', handleVisibilityChange);
});

onUnmounted(() => {
  window.removeEventListener('beforeunload', handleUnload);
  document.removeEventListener('visibilitychange', handleVisibilityChange);
});
</script>

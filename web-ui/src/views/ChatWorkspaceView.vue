<template>
  <div class="chat-workspace">
    <div class="chat-workspace__toolbar">
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
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue';
import PaneSplit from '../components/chat/PaneSplit.vue';
import LayoutShapeIcon from '../components/chat/LayoutShapeIcon.vue';
import SourcePanel from '../components/SourcePanel.vue';
import { useChatWorkspaceStore } from '../stores/chatWorkspace.js';

defineProps({ projectId: { type: String, default: null } });

const workspace = useChatWorkspaceStore();

const layoutsOpen   = ref(false);
const newLayoutName = ref('');

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

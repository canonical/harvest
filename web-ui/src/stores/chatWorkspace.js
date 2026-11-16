import { defineStore } from 'pinia';
import { ref, watch, computed } from 'vue';
import { createTree, allTabIds } from '../lib/pane-layout.js';
import { createLayoutOperations } from '../lib/layout-workspace.js';
import {
  getCurrentChatLayout, saveCurrentChatLayout, CHAT_LAYOUTS_URL,
  listChatLayouts as apiListChatLayouts, createChatLayout, getChatLayout,
  updateChatLayout, deleteChatLayout,
} from '../lib/api.js';
import { disposeChatInstance } from '../composables/useChatInstance.js';
import { useProjectStore } from './project.js';
import { useSpaceStore } from './space.js';

const AUTOSAVE_DEBOUNCE_MS = 1000;

export const useChatWorkspaceStore = defineStore('chatWorkspace', () => {
  const project = useProjectStore();
  const space = useSpaceStore();

  const tree = ref(createTree());
  const namedLayouts = ref([]);
  const currentNamedLayoutId = ref(null);
  const savedTreeSnapshot = ref(null);

  let autosaveTimer = null;

  function _snapshotTree() {
    savedTreeSnapshot.value = JSON.parse(JSON.stringify(tree.value));
  }

  function _treeMatchesSnapshot() {
    if (!savedTreeSnapshot.value) return false;
    return JSON.stringify(tree.value) === JSON.stringify(savedTreeSnapshot.value);
  }

  const isDefaultLayout = computed(() => {
    const root = tree.value.root;
    return root.type === 'leaf' && root.tabs.length === 1 && !root.tabs[0].conversationId;
  });

  const isLayoutSaved = computed(() => {
    if (isDefaultLayout.value) return true;
    return currentNamedLayoutId.value !== null && _treeMatchesSnapshot();
  });

  function _scheduleAutosave() {
    if (autosaveTimer) clearTimeout(autosaveTimer);
    autosaveTimer = setTimeout(() => {
      autosaveTimer = null;
      saveCurrentChatLayout(tree.value, space.projectId).catch(() => {});
    }, AUTOSAVE_DEBOUNCE_MS);
  }

  function flushAutosaveNow(projectId = space.projectId) {
    if (autosaveTimer) {
      clearTimeout(autosaveTimer);
      autosaveTimer = null;
    }
    try {
      const q = projectId ? `?project_id=${encodeURIComponent(projectId)}` : '';
      fetch(`${CHAT_LAYOUTS_URL}/current${q}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ tree: tree.value }),
        keepalive: true,
      });
    } catch {}
  }

  function disposeAllInstances() {
    for (const tabId of allTabIds(tree.value.root)) disposeChatInstance(tabId);
  }

  async function initFromServer() {
    await project.whenReady();
    tree.value = createTree({ projectId: space.projectId });
    try {
      const data = await getCurrentChatLayout(space.projectId);
      if (data?.tree) tree.value = data.tree;
    } catch {}
  }

  watch(() => space.projectId, async (newId, oldId) => {
    if (newId === oldId) return;
    flushAutosaveNow(oldId);
    disposeAllInstances();
    currentNamedLayoutId.value = null;
    namedLayouts.value = [];
    await initFromServer();
  });

  const operations = createLayoutOperations({
    tree,
    changed: _scheduleAutosave,
    emptyTab: () => ({ projectId: space.projectId }),
    disposeTab: (tab) => disposeChatInstance(tab.tabId),
  });

  async function listNamedLayouts() {
    try {
      namedLayouts.value = await apiListChatLayouts(space.projectId);
    } catch {
      namedLayouts.value = [];
    }
    return namedLayouts.value;
  }

  async function saveNamedLayout(name) {
    if (currentNamedLayoutId.value) {
      await updateChatLayout(currentNamedLayoutId.value, { name, tree: tree.value });
    } else {
      const created = await createChatLayout(name, tree.value, space.projectId);
      currentNamedLayoutId.value = created.id;
    }
    _snapshotTree();
    await listNamedLayouts();
  }

  async function loadNamedLayout(id) {
    const data = await getChatLayout(id);
    if (data?.tree) {
      disposeAllInstances();
      tree.value = data.tree;
      currentNamedLayoutId.value = id;
      _snapshotTree();
      _scheduleAutosave();
    }
  }

  function newLayout() {
    disposeAllInstances();
    tree.value = createTree({ projectId: space.projectId });
    currentNamedLayoutId.value = null;
    _snapshotTree();
    _scheduleAutosave();
  }

  async function deleteNamedLayout(id) {
    await deleteChatLayout(id);
    if (currentNamedLayoutId.value === id) currentNamedLayoutId.value = null;
    await listNamedLayouts();
  }

  return {
    tree, namedLayouts, currentNamedLayoutId, isLayoutSaved,
    initFromServer, flushAutosaveNow,
    ...operations,
    listNamedLayouts, saveNamedLayout, loadNamedLayout, deleteNamedLayout, newLayout,
  };
});

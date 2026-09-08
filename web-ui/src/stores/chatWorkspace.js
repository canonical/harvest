import { defineStore } from 'pinia';
import { ref, watch } from 'vue';
import {
  createTree, createTab, findPane, findTab, allTabIds,
  splitPane as splitPaneTree, removeTabFromPane as removeTabFromPaneTree,
  removePane as removePaneTree, moveTab as moveTabTree, setSizes,
} from '../lib/pane-layout.js';
import {
  getCurrentChatLayout, saveCurrentChatLayout, CHAT_LAYOUTS_URL,
  listChatLayouts as apiListChatLayouts, createChatLayout, getChatLayout,
  updateChatLayout, deleteChatLayout,
} from '../lib/api.js';
import { disposeChatInstance } from '../composables/useChatInstance.js';
import { useProjectStore } from './project.js';

const AUTOSAVE_DEBOUNCE_MS = 1000;

export const useChatWorkspaceStore = defineStore('chatWorkspace', () => {
  const project = useProjectStore();

  const tree = ref(createTree());
  const namedLayouts = ref([]);
  const currentNamedLayoutId = ref(null);

  let autosaveTimer = null;
  function _scheduleAutosave() {
    if (autosaveTimer) clearTimeout(autosaveTimer);
    autosaveTimer = setTimeout(() => {
      autosaveTimer = null;
      saveCurrentChatLayout(tree.value, project.selectedProjectId).catch(() => {});
    }, AUTOSAVE_DEBOUNCE_MS);
  }

  function flushAutosaveNow(projectId = project.selectedProjectId) {
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
    tree.value = createTree({ projectId: project.selectedProjectId });
    try {
      const data = await getCurrentChatLayout(project.selectedProjectId);
      if (data?.tree) tree.value = data.tree;
    } catch {}
  }

  watch(() => project.selectedProjectId, async (newId, oldId) => {
    if (newId === oldId) return;
    flushAutosaveNow(oldId);
    disposeAllInstances();
    currentNamedLayoutId.value = null;
    namedLayouts.value = [];
    await initFromServer();
  });

  function splitPaneWithTab(paneId, direction, tab) {
    tree.value = splitPaneTree(tree.value, paneId, direction, tab);
    _scheduleAutosave();
  }

  function moveTab(fromPaneId, tabId, toPaneId, index = -1) {
    tree.value = moveTabTree(tree.value, fromPaneId, tabId, toPaneId, index);
    _scheduleAutosave();
  }

  function moveTabToNewSplit(fromPaneId, tabId, toPaneId, edge) {
    const fromLoc = findPane(tree.value.root, fromPaneId);
    if (!fromLoc || fromLoc.node.type !== 'leaf') return;
    if (fromPaneId === toPaneId && fromLoc.node.tabs.length <= 1) return;
    const tabEntry = fromLoc.node.tabs.find(t => t.tabId === tabId);
    if (!tabEntry) return;
    tree.value = removeTabFromPaneTree(tree.value, fromPaneId, tabId);
    tree.value = splitPaneTree(tree.value, toPaneId, edge, tabEntry);
    _scheduleAutosave();
  }

  function closeTab(paneId, tabId) {
    tree.value = removeTabFromPaneTree(tree.value, paneId, tabId, { projectId: project.selectedProjectId });
    disposeChatInstance(tabId);
    _scheduleAutosave();
  }

  function closePane(paneId) {
    const loc = findPane(tree.value.root, paneId);
    if (loc?.node?.type === 'leaf') {
      for (const t of loc.node.tabs) disposeChatInstance(t.tabId);
    }
    tree.value = removePaneTree(tree.value, paneId, { projectId: project.selectedProjectId });
    _scheduleAutosave();
  }

  function setActiveTab(paneId, tabId) {
    const loc = findPane(tree.value.root, paneId);
    if (loc?.node?.type === 'leaf') {
      loc.node.activeTabId = tabId;
      _scheduleAutosave();
    }
  }

  function resizeSplit(splitId, sizes) {
    setSizes(tree.value, splitId, sizes);
    _scheduleAutosave();
  }

  function openTabInPane(paneId, { conversationId = null, projectId = null, title = 'New chat' } = {}) {
    const loc = findPane(tree.value.root, paneId);
    if (!loc || loc.node.type !== 'leaf') return null;
    const tab = createTab({ conversationId, projectId, title });
    loc.node.tabs.push(tab);
    loc.node.activeTabId = tab.tabId;
    _scheduleAutosave();
    return tab.tabId;
  }

  function updateTab(tabId, patch) {
    const found = findTab(tree.value.root, tabId);
    if (!found) return;
    Object.assign(found.tab, patch);
    _scheduleAutosave();
  }

  async function listNamedLayouts() {
    try {
      namedLayouts.value = await apiListChatLayouts(project.selectedProjectId);
    } catch {
      namedLayouts.value = [];
    }
    return namedLayouts.value;
  }

  async function saveNamedLayout(name) {
    if (currentNamedLayoutId.value) {
      await updateChatLayout(currentNamedLayoutId.value, { name, tree: tree.value });
    } else {
      const created = await createChatLayout(name, tree.value, project.selectedProjectId);
      currentNamedLayoutId.value = created.id;
    }
    await listNamedLayouts();
  }

  async function loadNamedLayout(id) {
    const data = await getChatLayout(id);
    if (data?.tree) {
      disposeAllInstances();
      tree.value = data.tree;
      currentNamedLayoutId.value = id;
      _scheduleAutosave();
    }
  }

  async function deleteNamedLayout(id) {
    await deleteChatLayout(id);
    if (currentNamedLayoutId.value === id) currentNamedLayoutId.value = null;
    await listNamedLayouts();
  }

  return {
    tree, namedLayouts, currentNamedLayoutId,
    initFromServer, flushAutosaveNow,
    splitPaneWithTab, moveTab, moveTabToNewSplit, closeTab, closePane, setActiveTab, resizeSplit,
    openTabInPane, updateTab,
    listNamedLayouts, saveNamedLayout, loadNamedLayout, deleteNamedLayout,
  };
});

import { inject, provide } from 'vue';
import {
  findPane, findTab, splitPane, removeTabFromPane, removePane, moveTab as moveTabInTree, setSizes,
} from './pane-layout.js';

const LAYOUT_WORKSPACE = Symbol('layoutWorkspace');

const OPERATIONS = [
  'openTab', 'splitPaneWithTab', 'moveTab', 'moveTabToNewSplit', 'closeTab', 'closePane', 'setActiveTab', 'resizeSplit', 'updateTab',
];

export function operationsOf(source) {
  return Object.fromEntries(OPERATIONS.map(name => [name, (...args) => source[name](...args)]));
}

export function provideLayoutWorkspace(workspace) {
  provide(LAYOUT_WORKSPACE, workspace);
}

export function useLayoutWorkspace() {
  const workspace = inject(LAYOUT_WORKSPACE, null);
  if (!workspace) throw new Error('a layout workspace must be provided');
  return workspace;
}

export function createLayoutOperations({ tree, changed, emptyTab = () => ({}), disposeTab = () => {} }) {
  function leaf(paneId) {
    const loc = findPane(tree.value.root, paneId);
    return loc?.node?.type === 'leaf' ? loc.node : null;
  }

  function openTab(paneId, tab) {
    const pane = leaf(paneId);
    if (!pane) return null;
    pane.tabs.push(tab);
    pane.activeTabId = tab.tabId;
    changed();
    return tab.tabId;
  }

  function splitPaneWithTab(paneId, edge, tab) {
    tree.value = splitPane(tree.value, paneId, edge, tab);
    changed();
  }

  function moveTab(fromPaneId, tabId, toPaneId, index = -1) {
    tree.value = moveTabInTree(tree.value, fromPaneId, tabId, toPaneId, index);
    changed();
  }

  function moveTabToNewSplit(fromPaneId, tabId, toPaneId, edge) {
    const from = leaf(fromPaneId);
    if (!from) return;
    if (fromPaneId === toPaneId && from.tabs.length <= 1) return;
    const tab = from.tabs.find(t => t.tabId === tabId);
    if (!tab) return;
    tree.value = removeTabFromPane(tree.value, fromPaneId, tabId);
    tree.value = splitPane(tree.value, toPaneId, edge, tab);
    changed();
  }

  function closeTab(paneId, tabId) {
    const tab = leaf(paneId)?.tabs.find(t => t.tabId === tabId);
    tree.value = removeTabFromPane(tree.value, paneId, tabId, emptyTab());
    if (tab) disposeTab(tab);
    changed();
  }

  function closePane(paneId) {
    for (const tab of leaf(paneId)?.tabs ?? []) disposeTab(tab);
    tree.value = removePane(tree.value, paneId, emptyTab());
    changed();
  }

  function setActiveTab(paneId, tabId) {
    const pane = leaf(paneId);
    if (!pane) return;
    pane.activeTabId = tabId;
    changed();
  }

  function resizeSplit(splitId, sizes) {
    setSizes(tree.value, splitId, sizes);
    changed();
  }

  function updateTab(tabId, patch) {
    const found = findTab(tree.value.root, tabId);
    if (!found) return;
    Object.assign(found.tab, patch);
    changed();
  }

  return {
    openTab, splitPaneWithTab, moveTab, moveTabToNewSplit, closeTab, closePane, setActiveTab, resizeSplit, updateTab,
  };
}

function makeId() {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) return crypto.randomUUID();
  return `id-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function evenSizes(n) {
  return Array.from({ length: n }, () => 1 / n);
}

export function createTab(overrides = {}) {
  return { tabId: makeId(), conversationId: null, projectId: null, title: 'New chat', ...overrides };
}

export function createLeaf(tabs, activeTabId) {
  const leafTabs = tabs && tabs.length ? tabs : [createTab()];
  return { type: 'leaf', id: makeId(), activeTabId: activeTabId ?? leafTabs[0].tabId, tabs: leafTabs };
}

export function createSplit(direction, children) {
  return { type: 'split', id: makeId(), direction, sizes: evenSizes(children.length), children };
}

export function createTree(tabOverrides = {}) {
  return { version: 1, root: createLeaf([createTab(tabOverrides)]) };
}

export function allTabIds(root) {
  if (root.type === 'leaf') return root.tabs.map(t => t.tabId);
  return root.children.flatMap(allTabIds);
}

export function layoutRects(root, box = { x: 0, y: 0, w: 100, h: 100 }) {
  const rects = [];

  function walk(node, x, y, w, h) {
    if (!node) return;
    if (node.type === 'leaf') {
      rects.push({ x, y, w, h, tabCount: node.tabs?.length ?? 1 });
      return;
    }
    const n = node.children.length;
    const sizes = node.sizes?.length === n ? node.sizes : evenSizes(n);
    let offset = 0;
    node.children.forEach((child, i) => {
      const frac = sizes[i] ?? 1 / n;
      if (node.direction === 'row') {
        const cw = w * frac;
        walk(child, x + offset, y, cw, h);
        offset += cw;
      } else {
        const ch = h * frac;
        walk(child, x, y + offset, w, ch);
        offset += ch;
      }
    });
  }

  walk(root, box.x, box.y, box.w, box.h);
  return rects;
}

export function findPane(root, paneId) {
  if (root.id === paneId) return { node: root, parent: null, index: -1 };
  if (root.type !== 'split') return null;
  for (let i = 0; i < root.children.length; i++) {
    const child = root.children[i];
    if (child.id === paneId) return { node: child, parent: root, index: i };
    const found = findPane(child, paneId);
    if (found) return found;
  }
  return null;
}

export function findTab(root, tabId) {
  if (root.type === 'leaf') {
    const tab = root.tabs.find(t => t.tabId === tabId);
    return tab ? { tab, pane: root } : null;
  }
  for (const child of root.children) {
    const found = findTab(child, tabId);
    if (found) return found;
  }
  return null;
}

export function setSizes(tree, splitId, sizes) {
  const loc = findPane(tree.root, splitId);
  if (loc?.node?.type === 'split') loc.node.sizes = sizes;
  return tree;
}

const EDGE_DIRECTION = { left: 'row', right: 'row', top: 'column', bottom: 'column' };
const EDGE_IS_BEFORE  = { left: true,  right: false, top: true,    bottom: false };

export function splitPane(tree, paneId, edge, newTab) {
  const loc = findPane(tree.root, paneId);
  if (!loc) return tree;
  const { node: pane, parent, index } = loc;
  const dir    = EDGE_DIRECTION[edge];
  const before = EDGE_IS_BEFORE[edge];
  const newLeaf = createLeaf([newTab], newTab.tabId);

  if (parent && parent.direction === dir) {
    const insertAt = before ? index : index + 1;
    parent.children.splice(insertAt, 0, newLeaf);
    parent.sizes = evenSizes(parent.children.length);
    return tree;
  }

  const children = before ? [newLeaf, pane] : [pane, newLeaf];
  const newSplit = createSplit(dir, children);
  if (!parent) {
    tree.root = newSplit;
  } else {
    parent.children[index] = newSplit;
  }
  return tree;
}

function findParentOf(root, nodeId) {
  if (root.type !== 'split') return null;
  for (let i = 0; i < root.children.length; i++) {
    if (root.children[i].id === nodeId) return { parent: root, index: i };
    const found = findParentOf(root.children[i], nodeId);
    if (found) return found;
  }
  return null;
}

function collapseIfSingleChild(tree, split) {
  if (!split || split.type !== 'split' || split.children.length > 1) {
    if (split?.type === 'split') split.sizes = evenSizes(split.children.length);
    return;
  }
  const onlyChild = split.children[0];
  if (tree.root === split) {
    tree.root = onlyChild;
    return;
  }
  const parentLoc = findParentOf(tree.root, split.id);
  if (!parentLoc) return;
  parentLoc.parent.children[parentLoc.index] = onlyChild;
  collapseIfSingleChild(tree, parentLoc.parent);
}

function removeEmptyPaneIfNeeded(tree, loc, tabOverrides) {
  if (loc.node.tabs.length) return;
  if (!loc.parent) {
    tree.root = createLeaf([createTab(tabOverrides)]);
    return;
  }
  loc.parent.children.splice(loc.index, 1);
  collapseIfSingleChild(tree, loc.parent);
}

export function removeTabFromPane(tree, paneId, tabId, tabOverrides) {
  const loc = findPane(tree.root, paneId);
  if (!loc || loc.node.type !== 'leaf') return tree;
  const pane = loc.node;

  const tabIdx = pane.tabs.findIndex(t => t.tabId === tabId);
  if (tabIdx === -1) return tree;
  pane.tabs.splice(tabIdx, 1);

  if (pane.tabs.length) {
    if (pane.activeTabId === tabId) {
      pane.activeTabId = pane.tabs[Math.min(tabIdx, pane.tabs.length - 1)].tabId;
    }
    return tree;
  }

  removeEmptyPaneIfNeeded(tree, loc, tabOverrides);
  return tree;
}

export function removePane(tree, paneId, tabOverrides) {
  const loc = findPane(tree.root, paneId);
  if (!loc || loc.node.type !== 'leaf') return tree;
  loc.node.tabs = [];
  removeEmptyPaneIfNeeded(tree, loc, tabOverrides);
  return tree;
}

export function moveTab(tree, fromPaneId, tabId, toPaneId, toIndex = -1) {
  const fromLoc = findPane(tree.root, fromPaneId);
  if (!fromLoc || fromLoc.node.type !== 'leaf') return tree;
  const fromPane = fromLoc.node;
  const tabIdx = fromPane.tabs.findIndex(t => t.tabId === tabId);
  if (tabIdx === -1) return tree;

  if (fromPaneId === toPaneId) {
    const [tab] = fromPane.tabs.splice(tabIdx, 1);
    const insertAt = toIndex < 0 || toIndex > fromPane.tabs.length ? fromPane.tabs.length : toIndex;
    fromPane.tabs.splice(insertAt, 0, tab);
    fromPane.activeTabId = tab.tabId;
    return tree;
  }

  const toLoc = findPane(tree.root, toPaneId);
  if (!toLoc || toLoc.node.type !== 'leaf') return tree;
  const toPane = toLoc.node;

  const [tab] = fromPane.tabs.splice(tabIdx, 1);
  const insertAt = toIndex < 0 || toIndex > toPane.tabs.length ? toPane.tabs.length : toIndex;
  toPane.tabs.splice(insertAt, 0, tab);
  toPane.activeTabId = tab.tabId;

  if (fromPane.tabs.length) {
    if (fromPane.activeTabId === tabId) {
      fromPane.activeTabId = fromPane.tabs[Math.min(tabIdx, fromPane.tabs.length - 1)].tabId;
    }
  } else {
    removeEmptyPaneIfNeeded(tree, fromLoc);
  }

  return tree;
}

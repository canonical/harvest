import cytoscape from 'cytoscape';
import { kindColor, kindShape, cytoscapeStyle } from './graph-utils.js';
import { openSourcePanel, closeSourcePanel } from './source-panel.js';
import { fetchGraph, queryStream, fetchToolDescription } from './api.js';
import { escapeHtml as esc } from './utils.js';

let cy = null;
let currentRepo = null;
let currentVersion = null;
let pendingGraphData = null;
let isPageVisible = false;
let repoList = [];
let activeLayoutWorker = null;
let activeSearchMatched = null;

const $ = id => document.getElementById(id);

export function initRepositoriesPage(repos) {
  repoList = repos;
  populateRepoSelect();
  bindEvents();
}

export function onRepositoriesPageShow() {
  isPageVisible = true;
  if (cy) {
    requestAnimationFrame(() => { cy.resize(); cy.fit(undefined, 40); });
  } else if (pendingGraphData) {
    const data = pendingGraphData;
    pendingGraphData = null;
    mountCy(data);
  }
}

export function onRepositoriesPageHide() {
  isPageVisible = false;
}

function populateRepoSelect() {
  const sel = $('repo-select');
  sel.innerHTML = '<option value="" disabled selected>Choose a repository</option>';
  repoList.forEach(r => {
    const opt = document.createElement('option');
    opt.value = r.name;
    opt.textContent = r.name;
    sel.appendChild(opt);
  });
}

function bindEvents() {
  $('repo-select').addEventListener('change', onRepoChange);
  $('version-select').addEventListener('change', onVersionChange);
  $('graph-search-btn').addEventListener('click', doSearch);
  $('graph-search').addEventListener('keydown', e => { if (e.key === 'Enter') doSearch(); });
  $('graph-search-clear').addEventListener('click', clearSearch);
  $('graph-fit-btn').addEventListener('click', () => cy?.fit(undefined, 40));
  $('source-panel-close').addEventListener('click', clearHighlight);
}

function onRepoChange() {
  currentRepo = $('repo-select').value;
  const verSel = $('version-select');
  verSel.innerHTML = '<option value="" disabled selected>Choose a version</option>';
  verSel.disabled = !currentRepo;
  if (!currentRepo) return;

  const repo = repoList.find(r => r.name === currentRepo);
  if (!repo) return;

  const sorted = [...repo.versions].sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
  sorted.forEach(v => {
    const opt = document.createElement('option');
    opt.value = v;
    opt.textContent = v;
    verSel.appendChild(opt);
  });

  if (sorted.length === 1) {
    verSel.value = sorted[0];
    onVersionChange();
  }
}

async function onVersionChange() {
  currentVersion = $('version-select').value;
  if (!currentVersion || !currentRepo) return;
  await loadGraph();
}

async function loadGraph() {
  setOverlay('loading');
  destroyCy();

  try {
    const data = await fetchGraph(currentRepo, currentVersion);
    if (!data.nodes.length) {
      setOverlay('empty', 'No symbols found for this version.');
      enableControls(false);
      return;
    }
    if (isPageVisible) {
      mountCy(data);
    } else {
      pendingGraphData = data;
      setOverlay('empty', 'Graph ready — navigate here to view.');
    }
    enableControls(true);
    if (data.truncated) {
      setStatus(
        `Showing ${data.nodes.length.toLocaleString()} of ${data.total_nodes.toLocaleString()} symbols ` +
        `— type definitions prioritised. Use AI search to locate specific functions.`
      );
    }
  } catch (err) {
    setOverlay('error', `Failed to load: ${esc(err.message)}`);
    enableControls(false);
  }
}

const positionKey = (repo, ver) => `harvest:positions:${repo}:${ver}`;

function savePositions() {
  if (!cy || !currentRepo || !currentVersion) return;
  try {
    const pos = {};
    cy.nodes('.symbol-node').forEach(n => {
      const p = n.position();
      pos[n.id()] = { x: Math.round(p.x), y: Math.round(p.y) };
    });
    localStorage.setItem(positionKey(currentRepo, currentVersion), JSON.stringify(pos));
  } catch { /* ignore */ }
}

function loadPositions(nodes) {
  try {
    const raw = localStorage.getItem(positionKey(currentRepo, currentVersion));
    if (!raw) return null;
    const pos = JSON.parse(raw);
    if (nodes.every(n => n.data.id in pos)) return pos;
    return null;
  } catch { return null; }
}

function adaptiveIterations(nodeCount) {
  if (nodeCount < 100)  return 2500;
  if (nodeCount < 300)  return 1000;
  if (nodeCount < 700)  return 500;
  return 250;
}

function adaptiveSpacing(nodeCount) {
  if (nodeCount < 50)  return { nodeRepulsion: 90000, idealEdgeLength: 1600, gravity: 0.02, tilingPadding: 500 };
  if (nodeCount < 150) return { nodeRepulsion: 60000, idealEdgeLength: 1100, gravity: 0.03, tilingPadding: 300 };
  if (nodeCount < 400) return { nodeRepulsion: 30000, idealEdgeLength: 600,  gravity: 0.08, tilingPadding: 150 };
  return                      { nodeRepulsion: 15000, idealEdgeLength: 320,  gravity: 0.15, tilingPadding: 80  };
}

function mountCy(data) {
  setOverlay('loading');

  const nodeElements = data.nodes.map(n => ({
    group: 'nodes',
    classes: `symbol-node kind-${(n.kind ?? 'unknown').toLowerCase()}`,
    data: {
      id: n.id,
      label: n.name,
      file: n.file,
      kind: n.kind,
      start_line: n.start_line,
      signature: n.signature ?? '',
      color: kindColor(n.kind),
      shape: kindShape(n.kind),
    },
  }));

  const edgeElements = data.edges.map(e => ({
    group: 'edges',
    data: { id: e.id, source: e.source, target: e.target, relation: e.relation },
  }));

  cy = cytoscape({
    container: $('cy'),
    elements: [...nodeElements, ...edgeElements],
    style: cytoscapeStyle(),
    minZoom: 0.04,
    maxZoom: 5,
  });

  cy.elements().style('opacity', 0);

  cy.on('tap', 'node.symbol-node', onNodeTap);
  cy.on('tap', e => { if (e.target === cy) { clearHighlight(); closeSourcePanel(); } });

  function revealGraph() {
    setOverlay('none');
    cy.elements().animate({ style: { opacity: 1 }, duration: 500, easing: 'ease-in-out' });
  }

  function applyPositions(positions) {
    cy.nodes().forEach(n => {
      const p = positions[n.id()];
      if (p) n.position(p);
    });
    cy.fit(undefined, 60);
    revealGraph();
  }

  const cached = loadPositions(nodeElements);

  if (cached) {
    cy.layout({ name: 'preset', positions: n => cached[n.id()] ?? { x: 0, y: 0 }, fit: true, padding: 60 }).run();
    revealGraph();
    return;
  }

  $('repo-loading-text').textContent = `Computing layout for ${nodeElements.length} symbols…`;

  const spacing = adaptiveSpacing(nodeElements.length);
  const fcoseOptions = {
    quality: 'default',
    randomize: true,
    nodeDimensionsIncludeLabels: true,
    uniformNodeDimensions: false,
    packComponents: true,
    tile: true,
    tilingPaddingVertical: spacing.tilingPadding,
    tilingPaddingHorizontal: spacing.tilingPadding,
    nodeRepulsion: spacing.nodeRepulsion,
    idealEdgeLength: spacing.idealEdgeLength,
    edgeElasticity: 0.45,
    numIter: adaptiveIterations(nodeElements.length),
    gravity: spacing.gravity,
    gravityRange: 3.5,
    initialEnergyOnIncremental: 0.3,
  };

  const worker = new Worker(new URL('./layout-worker.js', import.meta.url), { type: 'module' });
  activeLayoutWorker = worker;

  worker.onmessage = ({ data: { positions } }) => {
    if (activeLayoutWorker !== worker) return;
    activeLayoutWorker = null;
    worker.terminate();
    applyPositions(positions);
    savePositions();
  };

  worker.onerror = err => {
    if (activeLayoutWorker !== worker) return;
    activeLayoutWorker = null;
    worker.terminate();
    console.error('Layout worker error:', err);
    applyPositions({});
  };

  worker.postMessage({ elements: [...nodeElements, ...edgeElements], options: fcoseOptions });
}

function destroyCy() {
  if (activeLayoutWorker) { activeLayoutWorker.terminate(); activeLayoutWorker = null; }
  if (cy) { cy.destroy(); cy = null; }
  pendingGraphData = null;
  closeSourcePanel();
  clearSearch();
  clearStatus();
}

function onNodeTap(evt) {
  const node = evt.target;
  highlightNeighborhood(node);
  openSourcePanel(node.data(), currentRepo, currentVersion, buildRelationSections(node));
}

function buildRelationSections(node) {
  const chip = n => ({
    name: n.data('label'),
    title: n.data('file'),
    onClick: () => {
      highlightNeighborhood(n);
      openSourcePanel(n.data(), currentRepo, currentVersion, buildRelationSections(n));
      cy.animate({ center: { eles: n }, zoom: Math.max(cy.zoom(), 1.2), duration: 350, easing: 'ease-in-out' });
    },
  });

  return [
    { label: 'Extends',        chips: node.outgoers('edge[relation="inherits"]').targets().map(chip) },
    { label: 'Subclasses',     chips: node.incomers('edge[relation="inherits"]').sources().map(chip) },
    { label: 'Implements',     chips: node.outgoers('edge[relation="implements"]').targets().map(chip) },
    { label: 'Implemented by', chips: node.incomers('edge[relation="implements"]').sources().map(chip) },
    { label: 'Embeds',         chips: node.outgoers('edge[relation="embeds"]').targets().map(chip) },
    { label: 'Embedded by',    chips: node.incomers('edge[relation="embeds"]').sources().map(chip) },
    { label: 'Uses',           chips: node.outgoers('edge[relation="uses"]').targets().map(chip) },
    { label: 'Used by',        chips: node.incomers('edge[relation="uses"]').sources().map(chip) },
    { label: 'Member of',      chips: node.incomers('edge[relation="contains"]').sources().map(chip) },
    { label: 'Methods',        chips: node.outgoers('edge[relation="contains"]').targets().map(chip) },
    { label: 'Callers',        chips: node.incomers('edge[relation="calls"]').sources().map(chip) },
    { label: 'Callees',        chips: node.outgoers('edge[relation="calls"]').targets().map(chip) },
  ].filter(s => s.chips.length > 0);
}

function highlightNeighborhood(node) {
  cy.elements().addClass('dimmed').removeClass('active ring');
  const hood = node.closedNeighborhood();
  hood.removeClass('dimmed');
  hood.filter('.symbol-node').forEach(n => n.parent().removeClass('dimmed').addClass('ring'));
  node.addClass('ring');
  node.connectedEdges().addClass('active');
  node.select();
}

function clearHighlight() {
  if (!cy) return;
  if (activeSearchMatched) {
    restoreSearchHighlight();
  } else {
    cy.elements().removeClass('dimmed active ring').deselect();
  }
}

function restoreSearchHighlight() {
  if (!cy || !activeSearchMatched) return;
  cy.elements().addClass('dimmed').removeClass('ring active').deselect();
  const matched = cy.nodes('.symbol-node').filter(n => activeSearchMatched.has(n.id()));
  matched.closedNeighborhood().removeClass('dimmed');
  matched.forEach(n => { n.addClass('ring'); n.parent().removeClass('dimmed').addClass('ring'); });
}

async function doSearch() {
  const q = $('graph-search').value.trim();
  if (!q || !currentRepo || !currentVersion || !cy) return;

  $('graph-search-btn').disabled = true;
  $('graph-search').disabled = true;
  $('graph-search-btn').textContent = '…';

  const overlay = $('search-overlay');
  const callsContainer = $('search-overlay-calls');
  callsContainer.innerHTML = '';
  overlay.hidden = false;

  const pendingCalls = [];

  function addCall(name, input) {
    const div = document.createElement('div');
    div.className = 'search-overlay__call';

    const icon = document.createElement('i');
    icon.className = 'p-icon--circle-of-friends search-overlay__call-icon running';

    const text = document.createElement('span');
    text.className = 'search-overlay__call-text';
    text.textContent = name.replace(/_/g, ' ');

    div.append(icon, text);
    callsContainer.appendChild(div);
    const entry = { icon, text, done: false };
    pendingCalls.push(entry);
    return entry;
  }

  function stopSpin(entry) {
    entry.icon.classList.remove('running');
  }

  function completeCall() {
    const entry = pendingCalls.find(e => !e.done);
    if (!entry) return;
    entry.done = true;
    stopSpin(entry);
  }

  const prompt =
    `In repository "${currentRepo}" version "${currentVersion}", use the search_symbols tool ` +
    `to find functions and classes most related to: "${q}". Try relevant keyword variations.`;

  const matched = new Set();
  try {
    await queryStream(prompt, event => {
      if (event.type === 'tool_call') {
        const entry = addCall(event.name, event.input);
        fetchToolDescription(event.name, event.input).then(description => {
          if (description) {
            entry.text.textContent = description;
            stopSpin(entry);
          }
        });
      } else if (event.type === 'tool_result') {
        completeCall();
        if (event.name === 'search_symbols' && event.preview) {
          const parsed = tryParseJsonArray(event.preview);
          if (parsed) parsed.forEach(r => { if (r.name && r.file) matched.add(`${r.file}:${r.name}`); });
        }
      }
    });
  } catch (err) {
    overlay.hidden = true;
    setStatus(`Search error: ${esc(err.message)}`);
    resetSearchControls();
    return;
  }

  overlay.hidden = true;
  applySearchHighlight(matched, q);
  resetSearchControls();
}

function tryParseJsonArray(text) {
  if (!text) return null;
  try { const v = JSON.parse(text); return Array.isArray(v) ? v : null; } catch { /* fall through */ }
  const t = text.trimEnd().replace(/,?\s*$/, ']');
  try { const v = JSON.parse(t); return Array.isArray(v) ? v : null; } catch { return null; }
}

function applySearchHighlight(matchedIds, query) {
  if (!cy) return;
  if (!matchedIds.size) { setStatus(`No symbols found for "${esc(query)}"`); return; }

  activeSearchMatched = matchedIds;

  cy.elements().addClass('dimmed').removeClass('ring active');
  const matched = cy.nodes('.symbol-node').filter(n => matchedIds.has(n.id()));
  const hood = matched.closedNeighborhood();
  hood.removeClass('dimmed');
  matched.forEach(n => { n.addClass('ring'); n.parent().removeClass('dimmed').addClass('ring'); });

  if (matched.length) {
    cy.animate({ fit: { eles: hood, padding: 60 }, duration: 500, easing: 'ease-in-out' });
  }

  const count = matched.length;
  setStatus(`Found ${count} symbol${count !== 1 ? 's' : ''} matching "${esc(query)}"`);
  $('graph-search-clear').hidden = false;
}

function clearSearch() {
  activeSearchMatched = null;
  $('graph-search').value = '';
  $('graph-search-clear').hidden = true;
  clearStatus();
  if (cy) {
    cy.elements().removeClass('dimmed ring active').deselect();
    cy.animate({ fit: { padding: 40 }, duration: 400, easing: 'ease-in-out' });
  }
}

function resetSearchControls() {
  $('graph-search-btn').disabled = false;
  $('graph-search').disabled = false;
  $('graph-search-btn').textContent = 'Search';
}

function setOverlay(state, msg = '') {
  const loading = $('repo-loading');
  const empty = $('repo-empty');
  loading.hidden = state !== 'loading';
  empty.hidden = state === 'loading' || state === 'none';
  $('graph-legend').hidden = state !== 'none';
  if (state === 'empty' || state === 'error') {
    empty.querySelector('.repo-empty__text').innerHTML =
      state === 'error'
        ? `<span class="repo-empty__error">${msg}</span>`
        : msg || 'Select a repository and version<br>to explore its symbol graph';
  } else if (state === 'none') {
    empty.querySelector('.repo-empty__text').innerHTML =
      'Select a repository and version<br>to explore its symbol graph';
  }
}

function enableControls(on) {
  $('graph-search').disabled = !on;
  $('graph-search-btn').disabled = !on;
  $('graph-fit-btn').disabled = !on;
}

function setStatus(msg) {
  const el = $('repo-search-status');
  el.innerHTML = msg;
  el.hidden = false;
}

function clearStatus() {
  $('repo-search-status').hidden = true;
}

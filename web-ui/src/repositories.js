import cytoscape from 'cytoscape';
import fcose from 'cytoscape-fcose';
import hljs from 'highlight.js';
import { fetchGraph, fetchSymbolSource, queryStream, fetchToolDescription } from './api.js';

cytoscape.use(fcose);

// ── Kind → UML color ──────────────────────────────────────────────────────────

const KIND_COLORS = {
  class:     '#7C3AED',
  struct:    '#6D28D9',
  trait:     '#0891B2',
  interface: '#0284C7',
  function:  '#2563EB',
  method:    '#1D4ED8',
  enum:      '#D97706',
  module:    '#6B7280',
  impl:      '#059669',
  type:      '#DB2777',
};

function kindColor(kind) {
  return KIND_COLORS[kind?.toLowerCase()] ?? '#4F46E5';
}

// Shape: type-like nodes get a rounded rectangle, callables a plain rectangle
function kindShape(kind) {
  const k = kind?.toLowerCase() ?? '';
  if (k === 'class' || k === 'struct' || k === 'trait' || k === 'interface' || k === 'enum') {
    return 'rectangle';
  }
  return 'round-rectangle';
}

// ── State ─────────────────────────────────────────────────────────────────────

let cy = null;
let currentRepo = null;
let currentVersion = null;
let pendingGraphData = null;
let isPageVisible = false;
let repoList = [];
let activeLayoutWorker = null; // terminated if the user navigates away mid-layout
let activeSearchMatched = null; // Set of node IDs from the last AI search, or null

// ── DOM helpers ───────────────────────────────────────────────────────────────

const $ = id => document.getElementById(id);

function esc(str) {
  return String(str)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;')
    .replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;');
}

// ── Public API ────────────────────────────────────────────────────────────────

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

// ── Setup ─────────────────────────────────────────────────────────────────────

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
  $('source-panel-close').addEventListener('click', () => { closeSourcePanel(); clearHighlight(); });
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

// ── Graph loading ─────────────────────────────────────────────────────────────

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

// ── Layout position cache (localStorage) ─────────────────────────────────────
// Saves fcose-computed positions so revisiting a graph is instant.

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
  } catch { /* localStorage full or unavailable — non-fatal */ }
}

function loadPositions(nodes) {
  try {
    const raw = localStorage.getItem(positionKey(currentRepo, currentVersion));
    if (!raw) return null;
    const pos = JSON.parse(raw);
    // Only use cached positions if every node has a saved position.
    if (nodes.every(n => n.data.id in pos)) return pos;
    return null; // stale cache (e.g. after reingest added new symbols)
  } catch { return null; }
}

// ── Graph mounting ────────────────────────────────────────────────────────────

/** Scale layout iterations down for large graphs to keep the worker responsive. */
function adaptiveIterations(nodeCount) {
  if (nodeCount < 100)  return 2500;
  if (nodeCount < 300)  return 1000;
  if (nodeCount < 700)  return 500;
  return 250;
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

  /** Apply a precomputed position map and reveal the graph. */
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
    // ── Fast path: cached positions → instant preset layout ──────────────────
    cy.layout({ name: 'preset', positions: n => cached[n.id()] ?? { x: 0, y: 0 }, fit: true, padding: 60 }).run();
    revealGraph();
    return;
  }

  // ── Worker path: compute fcose off the main thread ────────────────────────
  // The main thread stays responsive; Firefox will not show a slow-script warning.
  $('repo-loading-text').textContent = `Computing layout for ${nodeElements.length} symbols…`;

  const fcoseOptions = {
    quality: 'default',
    randomize: true,
    nodeDimensionsIncludeLabels: true,
    uniformNodeDimensions: false,
    packComponents: true,
    tile: true,
    tilingPaddingVertical: 300,
    tilingPaddingHorizontal: 300,
    nodeRepulsion: 60000,
    idealEdgeLength: 1100,
    edgeElasticity: 0.45,
    numIter: adaptiveIterations(nodeElements.length),
    gravity: 0.03,
    gravityRange: 3.5,
    initialEnergyOnIncremental: 0.3,
  };

  const worker = new Worker(new URL('./layout-worker.js', import.meta.url), { type: 'module' });
  activeLayoutWorker = worker;

  worker.onmessage = ({ data: { positions } }) => {
    if (activeLayoutWorker !== worker) return; // stale — user navigated away
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
    // Fallback: random positions so the graph still renders
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

/** Remove the cached layout for the current repo/version (called after reingest). */
export function clearLayoutCache(repo, version) {
  try { localStorage.removeItem(positionKey(repo, version)); } catch { /* ignore */ }
}

// ── Cytoscape stylesheet ──────────────────────────────────────────────────────

function cytoscapeStyle() {
  return [
    // ── File container nodes ─────────────────────────────────────────────────
    {
      selector: 'node.file-node',
      style: {
        label: 'data(label)',
        'background-color': 'data(color)',
        'background-opacity': 0.06,
        'border-width': 1.5,
        'border-color': 'data(color)',
        'border-opacity': 0.55,
        color: 'data(color)',
        'font-size': '11px',
        'font-family': '"Ubuntu Mono", "Courier New", monospace',
        'font-weight': '600',
        'text-valign': 'top',
        'text-halign': 'center',
        'text-margin-y': -6,
        padding: '24px 16px 14px',
        'min-width': 60,
        'min-height': 32,
        'transition-property': 'opacity, border-opacity, background-opacity',
        'transition-duration': '180ms',
        cursor: 'default',
      },
    },
    {
      selector: 'node.file-node.dimmed',
      style: { opacity: 0.1 },
    },
    {
      selector: 'node.file-node.ring',
      style: {
        'border-opacity': 1,
        'border-width': 2.5,
        'background-opacity': 0.1,
      },
    },
    // ── Symbol nodes ─────────────────────────────────────────────────────────
    {
      selector: 'node.symbol-node',
      style: {
        label: 'data(label)',
        'background-color': 'data(color)',
        color: '#ffffff',
        'font-size': '11px',
        'font-family': '"Ubuntu Mono", "Courier New", monospace',
        'text-valign': 'center',
        'text-halign': 'center',
        'text-wrap': 'ellipsis',
        'text-max-width': '110px',
        shape: 'data(shape)',
        width: 'label',
        height: 'label',
        padding: '6px 11px',
        'min-width': 48,
        'min-height': 22,
        'border-width': 0,
        'border-color': '#ffffff',
        'transition-property': 'opacity, border-width',
        'transition-duration': '150ms',
        cursor: 'pointer',
      },
    },
    {
      selector: 'node.symbol-node:selected',
      style: { 'border-width': 3, 'border-color': '#ffffff' },
    },
    {
      selector: 'node.symbol-node.dimmed',
      style: { opacity: 0.1, color: 'transparent' },
    },
    {
      selector: 'node.symbol-node.ring',
      style: { 'border-width': 3, 'border-color': '#FFD700' },
    },
    // ── Edges — shared base ───────────────────────────────────────────────────
    {
      selector: 'edge',
      style: {
        'curve-style': 'bezier',
        'target-arrow-shape': 'none',
        'source-arrow-shape': 'none',
        'arrow-scale': 0.9,
        width: 1.5,
        'transition-property': 'opacity, line-color, target-arrow-color, source-arrow-color, width',
        'transition-duration': '150ms',
      },
    },
    // inherits: solid line, hollow triangle at parent (UML generalization)
    {
      selector: 'edge[relation="inherits"]',
      style: {
        'line-color': 'rgba(124,58,237,0.85)',
        'target-arrow-shape': 'triangle',
        'target-arrow-fill': 'hollow',
        'target-arrow-color': 'rgba(124,58,237,0.85)',
        'line-style': 'solid',
        width: 2,
      },
    },
    // implements: dashed line, hollow triangle at trait (UML realization)
    {
      selector: 'edge[relation="implements"]',
      style: {
        'line-color': 'rgba(8,145,178,0.85)',
        'target-arrow-shape': 'triangle',
        'target-arrow-fill': 'hollow',
        'target-arrow-color': 'rgba(8,145,178,0.85)',
        'line-style': 'dashed',
        'line-dash-pattern': [6, 3],
        width: 2,
      },
    },
    // embeds: solid line, open diamond at outer struct (UML aggregation)
    {
      selector: 'edge[relation="embeds"]',
      style: {
        'line-color': 'rgba(5,150,105,0.7)',
        'source-arrow-shape': 'diamond',
        'source-arrow-fill': 'hollow',
        'source-arrow-color': 'rgba(5,150,105,0.85)',
        'line-style': 'solid',
        width: 1.5,
      },
    },
    // contains: solid line, filled diamond at class (UML composition)
    {
      selector: 'edge[relation="contains"]',
      style: {
        'line-color': 'rgba(139,92,246,0.55)',
        'source-arrow-shape': 'diamond',
        'source-arrow-fill': 'filled',
        'source-arrow-color': 'rgba(139,92,246,0.75)',
        'line-style': 'solid',
        width: 1.5,
      },
    },
    // uses: thin solid arrow — struct/enum references another type in field declarations (UML association)
    {
      selector: 'edge[relation="uses"]',
      style: {
        'line-color': 'rgba(217,119,6,0.6)',
        'target-arrow-shape': 'triangle',
        'target-arrow-color': 'rgba(217,119,6,0.75)',
        'line-style': 'solid',
        width: 1,
      },
    },
    // calls: dashed line, arrow at callee (UML dependency)
    {
      selector: 'edge[relation="calls"]',
      style: {
        'line-color': 'rgba(120,120,120,0.4)',
        'target-arrow-shape': 'triangle',
        'target-arrow-color': 'rgba(120,120,120,0.55)',
        'line-style': 'dashed',
        'line-dash-pattern': [5, 3],
      },
    },
    {
      selector: 'edge.dimmed',
      style: { opacity: 0.04 },
    },
    {
      selector: 'edge[relation="inherits"].active',
      style: { 'line-color': '#7C3AED', 'target-arrow-color': '#7C3AED', width: 2.5, opacity: 1 },
    },
    {
      selector: 'edge[relation="implements"].active',
      style: { 'line-color': '#0891B2', 'target-arrow-color': '#0891B2', width: 2.5, opacity: 1 },
    },
    {
      selector: 'edge[relation="embeds"].active',
      style: { 'line-color': '#059669', 'source-arrow-color': '#059669', width: 2, opacity: 1 },
    },
    {
      selector: 'edge[relation="uses"].active',
      style: { 'line-color': '#D97706', 'target-arrow-color': '#D97706', width: 2, opacity: 1 },
    },
    {
      selector: 'edge[relation="contains"].active',
      style: {
        'line-color': '#8B5CF6',
        'source-arrow-color': '#8B5CF6',
        width: 2,
        opacity: 1,
      },
    },
    {
      selector: 'edge[relation="calls"].active',
      style: {
        'line-color': '#3B82F6',
        'target-arrow-color': '#3B82F6',
        width: 2.5,
        opacity: 1,
      },
    },
  ];
}

// ── Node interaction ──────────────────────────────────────────────────────────

async function onNodeTap(evt) {
  const node = evt.target;
  highlightNeighborhood(node);
  showSourcePanel(node.data());
  await loadSymbolSource(node.data());
  populateRelations(node);
}

function onFileNodeTap(evt) {
  // Clicking a file container highlights its children and their cross-file connections
  const fileNode = evt.target;
  const symbols = fileNode.children();
  if (!symbols.length) return;

  cy.elements().addClass('dimmed').removeClass('active ring');
  const neighborhood = symbols.closedNeighborhood();
  neighborhood.removeClass('dimmed');
  fileNode.removeClass('dimmed').addClass('ring');
  symbols.connectedEdges().addClass('active');
}

function highlightNeighborhood(node) {
  cy.elements().addClass('dimmed').removeClass('active ring');
  const hood = node.closedNeighborhood();
  hood.removeClass('dimmed');
  // Also un-dim the file containers of all highlighted symbols
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

// ── Source panel ──────────────────────────────────────────────────────────────

function showSourcePanel(data) {
  const badge = $('source-kind-badge');
  badge.textContent = data.kind.toLowerCase();
  badge.className = `source-kind-badge source-kind-badge--${data.kind.toLowerCase()}`;
  $('source-symbol-name').textContent = data.label;
  $('source-symbol-file').textContent = `${data.file}  ·  line ${data.start_line}`;
  $('source-code').textContent = '// Loading…';
  $('source-code').className = '';
  $('source-callers-section').hidden = true;
  $('source-callees-section').hidden = true;
  $('repo-source-panel').classList.add('is-open');
}

function closeSourcePanel() {
  $('repo-source-panel').classList.remove('is-open');
}

async function loadSymbolSource(data) {
  const codeEl = $('source-code');
  try {
    const sym = await fetchSymbolSource(currentRepo, currentVersion, data.file, data.label);
    if (!sym?.source) { codeEl.textContent = '// Source not available'; return; }
    const ext = data.file.split('.').pop().toLowerCase();
    const lang = { rs: 'rust', py: 'python', js: 'javascript', ts: 'typescript',
                   go: 'go', java: 'java', cpp: 'cpp', cc: 'cpp', c: 'c',
                   rb: 'ruby', sh: 'bash' }[ext];
    if (lang && hljs.getLanguage(lang)) {
      const result = hljs.highlight(sym.source, { language: lang, ignoreIllegals: true });
      codeEl.innerHTML = result.value;
      codeEl.className = `hljs language-${lang}`;
    } else {
      codeEl.textContent = sym.source;
    }
  } catch (err) {
    codeEl.textContent = `// Error: ${err.message}`;
  }
}

function populateRelations(node) {
  // Inheritance (Python, C++)
  renderChips($('source-parents-list'),  node.outgoers('edge[relation="inherits"]').targets(),  $('source-parents-section'));
  renderChips($('source-children-list'), node.incomers('edge[relation="inherits"]').sources(), $('source-children-section'));

  // Trait implementation (Rust)
  renderChips($('source-implements-list'),    node.outgoers('edge[relation="implements"]').targets(),  $('source-implements-section'));
  renderChips($('source-implementors-list'),  node.incomers('edge[relation="implements"]').sources(), $('source-implementors-section'));

  // Struct embedding (Go)
  renderChips($('source-embeds-list'),        node.outgoers('edge[relation="embeds"]').targets(),  $('source-embeds-section'));
  renderChips($('source-embedded-by-list'),   node.incomers('edge[relation="embeds"]').sources(), $('source-embedded-by-section'));

  // Field type references / UML association (Rust)
  renderChips($('source-uses-list'),    node.outgoers('edge[relation="uses"]').targets(),  $('source-uses-section'));
  renderChips($('source-used-by-list'), node.incomers('edge[relation="uses"]').sources(), $('source-used-by-section'));

  // Composition: type → methods
  renderChips($('source-container-list'), node.incomers('edge[relation="contains"]').sources(), $('source-container-section'));
  renderChips($('source-members-list'),   node.outgoers('edge[relation="contains"]').targets(), $('source-members-section'));

  // Call graph
  renderChips($('source-callers-list'), node.incomers('edge[relation="calls"]').sources(), $('source-callers-section'));
  renderChips($('source-callees-list'), node.outgoers('edge[relation="calls"]').targets(), $('source-callees-section'));
}

function renderChips(listEl, nodes, sectionEl) {
  if (!nodes.length) { sectionEl.hidden = true; return; }
  sectionEl.hidden = false;
  listEl.innerHTML = '';
  nodes.forEach(n => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'relation-chip';
    btn.textContent = n.data('label');
    btn.title = n.data('file');
    btn.addEventListener('click', () => {
      highlightNeighborhood(n);
      showSourcePanel(n.data());
      loadSymbolSource(n.data());
      populateRelations(n);
      cy.animate({ center: { eles: n }, zoom: Math.max(cy.zoom(), 1.2), duration: 350, easing: 'ease-in-out' });
    });
    listEl.appendChild(btn);
  });
}

// ── Smart search ──────────────────────────────────────────────────────────────

async function doSearch() {
  const q = $('graph-search').value.trim();
  if (!q || !currentRepo || !currentVersion || !cy) return;

  $('graph-search-btn').disabled = true;
  $('graph-search').disabled = true;
  $('graph-search-btn').textContent = '…';

  // Show the search progress overlay
  const overlay = $('search-overlay');
  const callsContainer = $('search-overlay-calls');
  callsContainer.innerHTML = '';
  overlay.hidden = false;

  // Track pending tool calls so tool_result can mark them done
  const pendingCalls = [];

  function addCall(name, input) {
    const div = document.createElement('div');
    div.className = 'search-overlay__call';

    const icon = document.createElement('i');
    // Spin until the AI-generated description arrives (same behaviour as chat)
    icon.className = 'p-icon--circle-of-friends search-overlay__call-icon running';

    const text = document.createElement('span');
    text.className = 'search-overlay__call-text';
    // Show humanised tool name as placeholder; replaced when description arrives
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

// ── Overlay / UI state ────────────────────────────────────────────────────────

// 'loading' | 'empty' | 'error' | 'none'
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

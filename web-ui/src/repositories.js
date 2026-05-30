import cytoscape from 'cytoscape';
import fcose from 'cytoscape-fcose';
import hljs from 'highlight.js';
import { fetchGraph, fetchSymbolSource, queryStream } from './api.js';

cytoscape.use(fcose);

// ── File → color palette ──────────────────────────────────────────────────────

const PALETTE = [
  '#2563EB', '#7C3AED', '#059669', '#D97706',
  '#DC2626', '#0891B2', '#DB2777', '#4F46E5',
  '#65A30D', '#EA580C',
];

const fileColorMap = new Map();
let fileColorIdx = 0;

function fileColor(file) {
  if (!fileColorMap.has(file)) {
    fileColorMap.set(file, PALETTE[fileColorIdx++ % PALETTE.length]);
  }
  return fileColorMap.get(file);
}

// Label shown on the file-container node (last 2 path components)
function fileLabel(path) {
  const parts = path.replace(/\\/g, '/').split('/');
  return parts.length >= 2 ? parts.slice(-2).join('/') : parts[0];
}

// ── State ─────────────────────────────────────────────────────────────────────

let cy = null;
let currentRepo = null;
let currentVersion = null;
let pendingGraphData = null;
let isPageVisible = false;
let repoList = [];

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
  sel.innerHTML = '<option value="">Repository…</option>';
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
  verSel.innerHTML = '<option value="">Version…</option>';
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
    if (data.truncated) setStatus('Showing first 300 symbols. Use search to explore further.');
  } catch (err) {
    setOverlay('error', `Failed to load: ${esc(err.message)}`);
    enableControls(false);
  }
}

// ── Graph mounting ────────────────────────────────────────────────────────────

function mountCy(data) {
  setOverlay('loading');
  fileColorMap.clear();
  fileColorIdx = 0;

  // Build unique file set and create compound-node parents
  const files = [...new Set(data.nodes.map(n => n.file))];

  const elements = [
    // File container nodes (parents)
    ...files.map(file => ({
      group: 'nodes',
      classes: 'file-node',
      data: {
        id: `file::${file}`,
        label: fileLabel(file),
        fullPath: file,
        color: fileColor(file),
      },
    })),
    // Symbol nodes (children of their file container)
    ...data.nodes.map(n => ({
      group: 'nodes',
      classes: 'symbol-node',
      data: {
        id: n.id,
        label: n.name,
        parent: `file::${n.file}`,
        file: n.file,
        kind: n.kind,
        start_line: n.start_line,
        signature: n.signature ?? '',
        color: fileColor(n.file),
        shape: n.kind === 'Class' ? 'ellipse' : 'round-rectangle',
      },
    })),
    // CALLS edges between symbol nodes
    ...data.edges.map(e => ({
      group: 'edges',
      data: { id: e.id, source: e.source, target: e.target },
    })),
  ];

  cy = cytoscape({
    container: $('cy'),
    elements,
    style: cytoscapeStyle(),
    minZoom: 0.04,
    maxZoom: 5,
  });

  // Start everything hidden for fade-in
  cy.elements().style('opacity', 0);

  const layout = cy.layout({
    name: 'fcose',
    quality: 'default',
    randomize: true,
    animate: true,
    animationDuration: 1000,
    animationEasing: 'ease-out',
    fit: true,
    padding: 60,
    // Include label extents so nodes never overlap their own text
    nodeDimensionsIncludeLabels: true,
    uniformNodeDimensions: false,
    // Pack isolated components (files with no cross-file calls) tightly
    packComponents: true,
    tile: true,
    tilingPaddingVertical: 16,
    tilingPaddingHorizontal: 16,
    // Edge and repulsion tuning
    idealEdgeLength: 160,
    edgeElasticity: 0.45,
    nestingFactor: 0.1,
    numIter: 2500,
    // Gravity: pull disconnected clusters toward center without crushing them
    gravity: 0.15,
    gravityRange: 3.8,
    gravityCompound: 1.0,
    gravityRangeCompound: 1.5,
    initialEnergyOnIncremental: 0.3,
  });

  // Use a one-shot guard that handles both the layout event and a fallback timer.
  // cy.one('layoutstop') can miss the event in some Cytoscape builds; layout.one()
  // is the authoritative listener.
  let revealed = false;
  function revealGraph() {
    if (revealed) return;
    revealed = true;
    setOverlay('none');
    cy.nodes('.symbol-node').animate({ style: { opacity: 1 }, duration: 500, easing: 'ease-in-out' });
    cy.nodes('.file-node').animate({ style: { opacity: 0.95 }, duration: 400, easing: 'ease-in-out' });
    cy.edges().animate({ style: { opacity: 0.55 }, duration: 700, easing: 'ease-in-out' });
  }

  layout.one('layoutstop', revealGraph);      // primary: fires on the layout object
  cy.one('layoutstop', revealGraph);           // secondary: cy-level event
  setTimeout(revealGraph, 3000);              // final fallback

  layout.run();

  cy.on('tap', 'node.symbol-node', onNodeTap);
  cy.on('tap', 'node.file-node', onFileNodeTap);
  cy.on('tap', e => { if (e.target === cy) { clearHighlight(); closeSourcePanel(); } });
}

function destroyCy() {
  if (cy) { cy.destroy(); cy = null; }
  fileColorMap.clear();
  fileColorIdx = 0;
  pendingGraphData = null;
  closeSourcePanel();
  clearSearch();
  clearStatus();
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
    // ── Edges ─────────────────────────────────────────────────────────────────
    {
      selector: 'edge',
      style: {
        'curve-style': 'bezier',
        'target-arrow-shape': 'triangle',
        'target-arrow-color': 'rgba(120,120,120,0.6)',
        'line-color': 'rgba(120,120,120,0.45)',
        width: 1.5,
        'arrow-scale': 0.75,
        'transition-property': 'opacity, line-color, target-arrow-color, width',
        'transition-duration': '150ms',
      },
    },
    {
      selector: 'edge.dimmed',
      style: { opacity: 0.04 },
    },
    {
      selector: 'edge.active',
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
  cy.elements().removeClass('dimmed active ring').deselect();
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
  renderChips($('source-callers-list'), node.incomers('node.symbol-node'), $('source-callers-section'));
  renderChips($('source-callees-list'), node.outgoers('node.symbol-node'), $('source-callees-section'));
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
  setStatus('Asking AI to find relevant symbols…');

  const prompt =
    `In repository "${currentRepo}" version "${currentVersion}", use the search_symbols tool ` +
    `to find functions and classes most related to: "${q}". Try relevant keyword variations.`;

  const matched = new Set();
  try {
    await queryStream(prompt, event => {
      if (event.type === 'tool_result' && event.name === 'search_symbols' && event.preview) {
        const parsed = tryParseJsonArray(event.preview);
        if (parsed) parsed.forEach(r => { if (r.name && r.file) matched.add(`${r.file}:${r.name}`); });
      }
    });
  } catch (err) {
    setStatus(`Search error: ${esc(err.message)}`);
    resetSearchControls();
    return;
  }

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
  if (state === 'empty' || state === 'error') {
    empty.querySelector('.repo-empty__text').innerHTML =
      state === 'error'
        ? `<span class="repo-empty__error">${msg}</span>`
        : msg || 'Select a repository and version<br>to explore its symbol graph';
  } else if (state === 'none') {
    // Ensure the initial empty-state text is restored for next fresh load
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

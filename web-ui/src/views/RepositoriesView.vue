<template>
  <div class="repositories-page">
    <div class="repo-toolbar">
      <div class="doc-selector">
        <label class="doc-selector__label">
          <svg width="13" height="13" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M2 2.5A2.5 2.5 0 0 1 4.5 0h8.75a.75.75 0 0 1 .75.75v12.5a.75.75 0 0 1-.75.75h-2.5a.75.75 0 0 1 0-1.5h1.75v-2h-8a1 1 0 0 0-.714 1.7.75.75 0 1 1-1.072 1.05A2.495 2.495 0 0 1 2 11.5Zm10.5-1h-8a1 1 0 0 0-1 1v6.708A2.486 2.486 0 0 1 4.5 9h8Z"/></svg>
          Repository
        </label>
        <select class="doc-select" v-model="selectedRepo" @change="onRepoChange">
          <option value="" disabled>Choose a repository</option>
          <option v-for="r in repos" :key="r.name" :value="r.name">{{ r.name }}</option>
        </select>
      </div>

      <svg class="doc-selector__sep" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="5.5 3.5 10.5 8 5.5 12.5"/></svg>

      <div class="doc-selector">
        <label class="doc-selector__label">
          <svg width="13" height="13" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M1 7.775V2.75C1 1.784 1.784 1 2.75 1h5.025c.464 0 .91.184 1.238.513l6.25 6.25a1.75 1.75 0 0 1 0 2.474l-5.026 5.026a1.75 1.75 0 0 1-2.474 0l-6.25-6.25A1.752 1.752 0 0 1 1 7.775Zm1.5 0c0 .066.026.13.073.177l6.25 6.25a.25.25 0 0 0 .354 0l5.025-5.025a.25.25 0 0 0 0-.354l-6.25-6.25a.25.25 0 0 0-.177-.073H2.75a.25.25 0 0 0-.25.25ZM6 5a1 1 0 1 1 0 2 1 1 0 0 1 0-2Z"/></svg>
          Version
        </label>
        <select class="doc-select" v-model="selectedVersion" :disabled="!selectedRepo" @change="onVersionChange">
          <option value="" disabled>Choose a version</option>
          <option v-for="v in sortedVersions" :key="v" :value="v">{{ v }}</option>
        </select>
      </div>

      <div class="repo-search-bar">
        <input
          v-model="searchQuery"
          type="search"
          placeholder="Smart search with AI…"
          autocomplete="off"
          :disabled="!cyReady"
          @keydown.enter="doSearch"
        />
        <button id="graph-search-btn" :disabled="!cyReady || searching" @click="doSearch">{{ searching ? '…' : 'Search' }}</button>
        <button v-if="searchActive" id="graph-search-clear" aria-label="Clear search" @click="clearSearch">✕</button>
      </div>

      <div class="repo-toolbar__actions">
        <button :disabled="!cyReady" @click="fitGraph">Fit</button>
      </div>
    </div>

    <p v-if="searchStatus" class="repo-search-status">{{ searchStatus }}</p>

    <div class="repo-main">
      <div class="repo-graph-wrap">
        <div v-if="graphLoading" class="repo-loading">
          <div class="loading-dots"><span></span><span></span><span></span></div>
          <span>{{ loadingText }}</span>
        </div>
        <div v-else-if="graphError" class="repo-empty">
          <span class="repo-empty__error">{{ graphError }}</span>
        </div>
        <div v-else-if="!cyReady && !selectedVersion" class="repo-empty">
          <div class="repo-empty__icon">
            <svg viewBox="0 0 16 16" fill="currentColor" xmlns="http://www.w3.org/2000/svg" aria-hidden="true"><path d="M.81 7.36a1.92 1.92 0 1 1 3.799.572A1.92 1.92 0 0 1 .81 7.36M8.826 3.033a1.92 1.92 0 1 1 3.755.806 1.92 1.92 0 0 1-3.755-.806M7.04 12.585a4.68 4.68 0 0 1-3.19-2.432 2.76 2.76 0 0 1-1.64.202 6.25 6.25 0 0 0 4.498 3.77c.45.098.908.144 1.364.141a2.74 2.74 0 0 1-.562-1.605 5 5 0 0 1-.47-.076M8.394 12.193a1.92 1.92 0 0 1 3.754.805 1.92 1.92 0 1 1-3.754-.805M12.943 11.89a6.3 6.3 0 0 0 1.22-2.587 6.3 6.3 0 0 0-.905-4.782 2.77 2.77 0 0 1-1.08 1.265 4.7 4.7 0 0 1-.154 4.674c.45.37.77.87.919 1.43M2.56 4.892a2.75 2.75 0 0 1 1.603.41 4.68 4.68 0 0 1 3.77-2.015q.012-.218.057-.433c.088-.411.268-.795.525-1.124A6.31 6.31 0 0 0 2.56 4.892"/></svg>
          </div>
          <p class="repo-empty__text">Select a repository and version<br>to explore its symbol graph</p>
        </div>

        <div v-if="searching" class="search-overlay">
          <div class="search-overlay__panel">
            <div class="search-overlay__heading">
              <div class="loading-dots"><span></span><span></span><span></span></div>
              <span>Searching for "<strong>{{ searchQuery }}</strong>"</span>
            </div>
            <div class="search-overlay__calls">
              <div v-for="(call, i) in searchCalls" :key="i" class="search-overlay__call">
                <svg v-if="call.done" class="search-overlay__call-icon done" viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <polyline points="3 8.5 6.5 12 13 4.5"/>
                </svg>
                <svg v-else class="search-overlay__call-icon running" viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
                  <circle cx="8" cy="8" r="6" stroke-opacity="0.2"/>
                  <path d="M8 2 A6 6 0 0 1 14 8" stroke-linecap="round"/>
                </svg>
                <span class="search-overlay__call-text">{{ call.label }}</span>
                <span class="search-overlay__call-status">
                  <template v-if="call.done && call.count !== null">{{ call.count === 0 ? 'no matches' : `${call.count} match${call.count !== 1 ? 'es' : ''}` }}</template>
                  <template v-else-if="!call.done">…</template>
                </span>
              </div>
            </div>
            <div v-if="searchFoundCount > 0" class="search-overlay__total">
              {{ searchFoundCount }} unique symbol{{ searchFoundCount !== 1 ? 's' : '' }} found
            </div>
          </div>
        </div>

        <div ref="cyEl" id="cy" class="repo-graph" />

        <div v-if="cyReady" class="graph-legend">
          <div v-for="(color, kind) in KIND_COLORS" :key="kind" class="graph-legend__item">
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
              <circle cx="5" cy="5" r="4" :fill="color" />
            </svg>
            <span>{{ kind }}</span>
          </div>
          <div class="graph-legend__divider"></div>
          <div class="graph-legend__item">
            <svg width="32" height="10" viewBox="0 0 32 10" aria-hidden="true">
              <line x1="0" y1="5" x2="25" y2="5" stroke="rgba(124,58,237,0.85)" stroke-width="1.8"/>
              <polygon points="25,2 32,5 25,8" fill="none" stroke="rgba(124,58,237,0.85)" stroke-width="1.3"/>
            </svg>
            <span>inherits</span>
          </div>
          <div class="graph-legend__item">
            <svg width="32" height="10" viewBox="0 0 32 10" aria-hidden="true">
              <line x1="0" y1="5" x2="25" y2="5" stroke="rgba(8,145,178,0.85)" stroke-width="1.8" stroke-dasharray="5,3"/>
              <polygon points="25,2 32,5 25,8" fill="none" stroke="rgba(8,145,178,0.85)" stroke-width="1.3"/>
            </svg>
            <span>implements</span>
          </div>
          <div class="graph-legend__item">
            <svg width="32" height="10" viewBox="0 0 32 10" aria-hidden="true">
              <polygon points="0,5 5,2 10,5 5,8" fill="none" stroke="rgba(5,150,105,0.85)" stroke-width="1.3"/>
              <line x1="10" y1="5" x2="32" y2="5" stroke="rgba(5,150,105,0.7)" stroke-width="1.5"/>
            </svg>
            <span>embeds</span>
          </div>
          <div class="graph-legend__item">
            <svg width="32" height="10" viewBox="0 0 32 10" aria-hidden="true">
              <polygon points="0,5 5,2 10,5 5,8" fill="rgba(139,92,246,0.75)" stroke="rgba(139,92,246,0.75)" stroke-width="1"/>
              <line x1="10" y1="5" x2="32" y2="5" stroke="rgba(139,92,246,0.55)" stroke-width="1.5"/>
            </svg>
            <span>contains</span>
          </div>
          <div class="graph-legend__item">
            <svg width="32" height="10" viewBox="0 0 32 10" aria-hidden="true">
              <line x1="0" y1="5" x2="25" y2="5" stroke="rgba(217,119,6,0.6)" stroke-width="1.5"/>
              <polygon points="25,2.5 32,5 25,7.5" fill="rgba(217,119,6,0.75)" stroke="none"/>
            </svg>
            <span>uses</span>
          </div>
          <div class="graph-legend__item">
            <svg width="32" height="10" viewBox="0 0 32 10" aria-hidden="true">
              <line x1="0" y1="5" x2="25" y2="5" stroke="rgba(120,120,120,0.55)" stroke-width="1.5" stroke-dasharray="4,3"/>
              <polygon points="25,2.5 32,5 25,7.5" fill="rgba(120,120,120,0.55)" stroke="none"/>
            </svg>
            <span>calls</span>
          </div>
        </div>
      </div>
    </div>

    <div class="repo-source-panel" :class="{ 'is-open': !!sourcePanel, 'is-expanded': panelExpanded }">
      <div class="repo-source-panel__body">
        <div class="repo-source-panel__head">
          <button
            class="source-panel-expand p-button--base"
            type="button"
            :aria-label="panelExpanded ? 'Shrink source panel' : 'Expand source panel'"
            @click="panelExpanded = !panelExpanded"
          >
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <polyline v-if="!panelExpanded" points="10.5 3.5 5.5 8 10.5 12.5"/>
              <polyline v-else               points="5.5 3.5 10.5 8 5.5 12.5"/>
            </svg>
          </button>
          <div class="repo-source-panel__title">
            <span class="source-symbol-name">{{ sourcePanel?.name }}</span>
            <span v-if="sourcePanel?.kind" class="source-kind-badge" :class="`source-kind-badge--${(sourcePanel.kind ?? '').toLowerCase()}`">{{ sourcePanel?.kind }}</span>
          </div>
          <button class="source-panel-close" type="button" aria-label="Close panel" @click="sourcePanel = null; panelExpanded = false">✕</button>
        </div>
        <p v-if="sourcePanel?.file" class="source-symbol-file">{{ sourcePanel.file }}:{{ sourcePanel.start_line }}</p>
        <div class="source-code-block">
          <div v-if="sourcePanel?.loading" class="source-loading">
            <div class="loading-dots"><span></span><span></span><span></span></div>
          </div>
          <pre v-else-if="sourcePanel?.highlightedCode"><code class="hljs" v-html="sourcePanel.highlightedCode"></code></pre>
          <p v-else class="source-no-source">No source available</p>
        </div>
        <div v-if="sourcePanel?.relations?.length" class="source-relations">
          <div v-for="section in sourcePanel.relations" :key="section.label" class="source-relations__section">
            <div class="source-relations__label">{{ section.label }}</div>
            <div class="source-relations__chips">
              <button
                v-for="chip in section.chips"
                :key="chip.name"
                class="relation-chip"
                :title="chip.title"
                @click="chip.onClick()"
              >{{ chip.name }}</button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue';
import cytoscape from 'cytoscape';
import fcose     from 'cytoscape-fcose';
import hljs from 'highlight.js';
import { kindColor, kindShape, cytoscapeStyle, KIND_COLORS } from '../lib/graph-utils.js';
import { fetchGraph, queryStream, fetchRepositories, fetchSymbolSource } from '../lib/api.js';

function guessLanguage(file) {
  if (!file) return 'plaintext';
  const ext = (file.split('.').pop() ?? '').toLowerCase();
  const map = {
    rs: 'rust', py: 'python', js: 'javascript', ts: 'typescript',
    jsx: 'javascript', tsx: 'typescript', go: 'go', java: 'java',
    rb: 'ruby', cpp: 'cpp', cc: 'cpp', cxx: 'cpp', c: 'c',
    h: 'c', cs: 'csharp', php: 'php', swift: 'swift', kt: 'kotlin',
    scala: 'scala', sh: 'bash', bash: 'bash', yml: 'yaml', yaml: 'yaml',
    json: 'json', toml: 'toml', html: 'html', css: 'css', sql: 'sql',
    lua: 'lua', hs: 'haskell', ex: 'elixir', exs: 'elixir', dart: 'dart',
  };
  const lang = map[ext] ?? ext;
  return hljs.getLanguage(lang) ? lang : 'plaintext';
}

function highlightCode(code, file) {
  try {
    return hljs.highlight(code, { language: guessLanguage(file) }).value;
  } catch {
    return hljs.highlight(code, { language: 'plaintext' }).value;
  }
}

cytoscape.use(fcose);

const repos = ref([]);

const selectedRepo    = ref('');
const selectedVersion = ref('');
const graphLoading    = ref(false);
const graphError      = ref(null);
const loadingText     = ref('Loading…');
const cyEl            = ref(null);
const cyReady         = ref(false);
const searchQuery     = ref('');
const searching       = ref(false);
const searchCalls     = ref([]);
const searchFoundCount = ref(0);
const searchStatus    = ref('');
const searchActive    = ref(false);
const sourcePanel     = ref(null);
const panelExpanded   = ref(false);

let cy                   = null;
let activeLayoutWorker   = null;
let activeSearchMatched  = null;

const sortedVersions = computed(() => {
  const r = repos.value.find(r => r.name === selectedRepo.value);
  if (!r) return [];
  return [...r.versions].sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
});

function positionKey(repo, ver) { return `harvest:positions:${repo}:${ver}`; }

function savePositions() {
  if (!cy || !selectedRepo.value || !selectedVersion.value) return;
  try {
    const pos = {};
    cy.nodes('.symbol-node').forEach(n => { const p = n.position(); pos[n.id()] = { x: Math.round(p.x), y: Math.round(p.y) }; });
    localStorage.setItem(positionKey(selectedRepo.value, selectedVersion.value), JSON.stringify(pos));
  } catch {}
}

function loadPositions(nodes) {
  try {
    const raw = localStorage.getItem(positionKey(selectedRepo.value, selectedVersion.value));
    if (!raw) return null;
    const pos = JSON.parse(raw);
    if (nodes.every(n => n.data.id in pos)) return pos;
    return null;
  } catch { return null; }
}

function destroyCy() {
  if (activeLayoutWorker) { activeLayoutWorker.terminate(); activeLayoutWorker = null; }
  if (cy) { cy.destroy(); cy = null; }
  cyReady.value  = false;
  sourcePanel.value = null;
  panelExpanded.value = false;
  clearSearch();
}

function onRepoChange() {
  selectedVersion.value = '';
  destroyCy();
  graphError.value = null;
}

async function onVersionChange() {
  if (!selectedVersion.value || !selectedRepo.value) return;
  destroyCy();
  graphLoading.value = true;
  graphError.value   = null;
  loadingText.value  = 'Loading graph…';
  try {
    const data = await fetchGraph(selectedRepo.value, selectedVersion.value);
    graphLoading.value = false;
    if (!data.nodes.length) {
      graphError.value = 'No symbols found for this version.';
      return;
    }
    mountCy(data);
    if (data.truncated) {
      searchStatus.value = `Showing ${data.nodes.length.toLocaleString()} of ${data.total_nodes.toLocaleString()} symbols.`;
    }
  } catch (e) {
    graphLoading.value = false;
    graphError.value   = `Failed to load: ${e.message}`;
  }
}

function mountCy(data) {
  if (!cyEl.value) return;

  const nodeEls = data.nodes.map(n => ({
    group: 'nodes',
    classes: `symbol-node kind-${(n.kind ?? 'unknown').toLowerCase()}`,
    data: {
      id: n.id, label: n.name, file: n.file, kind: n.kind,
      start_line: n.start_line, signature: n.signature ?? '',
      color: kindColor(n.kind), shape: kindShape(n.kind),
    },
  }));
  const edgeEls = data.edges.map(e => ({
    group: 'edges',
    data: { id: e.id, source: e.source, target: e.target, relation: e.relation },
  }));

  cy = cytoscape({
    container: cyEl.value,
    elements: [...nodeEls, ...edgeEls],
    style: cytoscapeStyle(),
    minZoom: 0.04, maxZoom: 5,
  });
  cy.elements().style('opacity', 0);
  cy.on('tap', 'node.symbol-node', (evt) => {
    const node = evt.target;
    highlightNeighborhood(node);
    openSymbolPanel(node);
  });
  cy.on('tap', e => {
    if (e.target === cy) {
      clearHighlight();
      sourcePanel.value = null;
      panelExpanded.value = false;
    }
  });

  const cached = loadPositions(nodeEls);
  if (cached) {
    cy.layout({ name: 'preset', positions: n => cached[n.id()] ?? { x: 0, y: 0 }, fit: true, padding: 60 }).run();
    revealGraph();
    return;
  }

  const n = nodeEls.length;
  loadingText.value = `Computing layout for ${n} symbols…`;
  graphLoading.value = true;

  const spacing = n < 50  ? { nodeRepulsion: 90000, idealEdgeLength: 1600, gravity: 0.02, tilingPadding: 500 }
                : n < 150 ? { nodeRepulsion: 60000, idealEdgeLength: 1100, gravity: 0.03, tilingPadding: 300 }
                : n < 400 ? { nodeRepulsion: 30000, idealEdgeLength: 600,  gravity: 0.08, tilingPadding: 150 }
                :           { nodeRepulsion: 15000, idealEdgeLength: 320,  gravity: 0.15, tilingPadding: 80  };

  const iterations = n < 100 ? 2500 : n < 300 ? 1000 : n < 700 ? 500 : 250;

  const worker = new Worker(new URL('../lib/layout-worker.js', import.meta.url), { type: 'module' });
  activeLayoutWorker = worker;
  worker.onmessage = ({ data: { positions } }) => {
    if (activeLayoutWorker !== worker) return;
    activeLayoutWorker = null;
    worker.terminate();
    graphLoading.value = false;
    applyPositions(positions);
    savePositions();
  };
  worker.onerror = () => {
    if (activeLayoutWorker !== worker) return;
    activeLayoutWorker = null;
    worker.terminate();
    graphLoading.value = false;
    applyPositions({});
  };
  worker.postMessage({ elements: [...nodeEls, ...edgeEls], options: { quality: 'default', randomize: true, nodeDimensionsIncludeLabels: true, uniformNodeDimensions: false, packComponents: true, tile: true, tilingPaddingVertical: spacing.tilingPadding, tilingPaddingHorizontal: spacing.tilingPadding, nodeRepulsion: spacing.nodeRepulsion, idealEdgeLength: spacing.idealEdgeLength, edgeElasticity: 0.45, numIter: iterations, gravity: spacing.gravity, gravityRange: 3.5, initialEnergyOnIncremental: 0.3 } });
}

function applyPositions(positions) {
  cy?.nodes().forEach(n => { const p = positions[n.id()]; if (p) n.position(p); });
  cy?.fit(undefined, 60);
  revealGraph();
}

function revealGraph() {
  cyReady.value = true;
  cy?.elements().animate({ style: { opacity: 1 }, duration: 500, easing: 'ease-in-out' });
}

function fitGraph() { cy?.fit(undefined, 40); }

function openSymbolPanel(node) {
  const name = node.data('label');
  const file = node.data('file');
  const rawSig = node.data('signature') || null;
  sourcePanel.value = {
    name,
    kind:            node.data('kind'),
    file,
    start_line:      node.data('start_line'),
    highlightedCode: rawSig ? highlightCode(rawSig, file) : null,
    relations:       buildRelations(node),
    loading:         true,
  };
  fetchSymbolSource(selectedRepo.value, selectedVersion.value, file, name)
    .then(src => {
      if (sourcePanel.value?.name === name && sourcePanel.value?.file === file) {
        const code = src?.source ?? src?.signature ?? rawSig;
        sourcePanel.value = {
          ...sourcePanel.value,
          highlightedCode: code ? highlightCode(code, file) : null,
          loading: false,
        };
      }
    })
    .catch(() => {
      if (sourcePanel.value?.name === name && sourcePanel.value?.file === file) {
        sourcePanel.value = { ...sourcePanel.value, loading: false };
      }
    });
}

function buildRelations(node) {
  const chip = n => ({
    name: n.data('label'),
    title: n.data('file'),
    onClick: () => {
      highlightNeighborhood(n);
      openSymbolPanel(n);
      cy?.animate({ center: { eles: n }, zoom: Math.max(cy.zoom(), 1.2), duration: 350, easing: 'ease-in-out' });
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
  cy?.elements().addClass('dimmed').removeClass('active ring');
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

function clearSearch() {
  activeSearchMatched = null;
  searchQuery.value  = '';
  searchActive.value = false;
  searchStatus.value = '';
  cy?.elements().removeClass('dimmed ring active').deselect();
  cy?.animate({ fit: { padding: 40 }, duration: 400, easing: 'ease-in-out' });
}

function tryParseJsonArray(text) {
  if (!text) return null;
  try { const v = JSON.parse(text); return Array.isArray(v) ? v : null; } catch {}
  try { const v = JSON.parse(text.trimEnd().replace(/,?\s*$/, ']')); return Array.isArray(v) ? v : null; } catch { return null; }
}

async function doSearch() {
  const q = searchQuery.value.trim();
  if (!q || !cy) return;
  searching.value    = true;
  searchCalls.value  = [];
  searchFoundCount.value = 0;
  searchStatus.value = '';

  const prompt = `In repository "${selectedRepo.value}" version "${selectedVersion.value}", use the search_symbols tool to find functions and classes most related to: "${q}". Try relevant keyword variations.`;
  const matched = new Set();

  try {
    await queryStream(prompt, null, [], (event) => {
      if (event.type === 'tool_call') {
        const keyword = event.input?.query ?? null;
        const kind    = event.input?.kind && event.input.kind !== 'any' ? event.input.kind : null;
        const label   = keyword
          ? `Searching for "${keyword}"${kind ? ` (${kind}s)` : ''}`
          : event.name.replace(/_/g, ' ');
        searchCalls.value.unshift({ label, count: null, done: false, isSearch: event.name === 'search_symbols' });
      } else if (event.type === 'tool_result') {
        const last = searchCalls.value.find(c => !c.done);
        if (last) last.done = true;
        if (event.name === 'search_symbols' && event.preview) {
          const parsed = tryParseJsonArray(event.preview);
          if (parsed) {
            const hits = parsed.filter(r => r.name && r.file);
            hits.forEach(r => matched.add(`${r.file}:${r.name}`));
            if (last) last.count = hits.length;
            searchFoundCount.value = matched.size;
          }
        }
      }
    });
  } catch (e) {
    searchStatus.value = `Search error: ${e.message}`;
    searching.value = false;
    return;
  }

  searching.value = false;
  applySearchHighlight(matched, q);
}

function applySearchHighlight(matchedIds, query) {
  if (!cy) return;
  if (!matchedIds.size) { searchStatus.value = `No symbols found for "${query}"`; return; }

  activeSearchMatched = matchedIds;
  searchActive.value  = true;

  cy.elements().addClass('dimmed').removeClass('ring active');
  const matched = cy.nodes('.symbol-node').filter(n => matchedIds.has(n.id()));
  const hood = matched.closedNeighborhood();
  hood.removeClass('dimmed');
  matched.forEach(n => { n.addClass('ring'); n.parent().removeClass('dimmed').addClass('ring'); });
  if (matched.length) cy.animate({ fit: { eles: hood, padding: 60 }, duration: 500, easing: 'ease-in-out' });

  searchStatus.value = `Found ${matched.length} symbol${matched.length !== 1 ? 's' : ''} matching "${query}"`;
}

onMounted(async () => {
  try { repos.value = await fetchRepositories(); } catch {}
});

onUnmounted(() => { destroyCy(); });
</script>

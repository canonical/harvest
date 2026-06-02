import cytoscape from 'cytoscape';
import { kindColor, kindShape, cytoscapeStyle } from './graph-utils.js';
import { openSourcePanel, closeSourcePanel } from './source-panel.js';

const RELATION_DEFS = [
  { rel: 'inherits',   out: 'Extends',         inc: 'Subclasses'     },
  { rel: 'implements', out: 'Implements',       inc: 'Implemented by' },
  { rel: 'embeds',     out: 'Embeds',           inc: 'Embedded by'    },
  { rel: 'uses',       out: 'Uses',             inc: 'Used by'        },
  { rel: 'contains',   out: 'Methods',          inc: 'Member of'      },
  { rel: 'calls',      out: 'Callees',          inc: 'Callers'        },
];

export function mountInlineGraphs(containerEl) {
  containerEl.querySelectorAll('.inline-graph[data-graph]').forEach(mountInlineGraph);
}

function mountInlineGraph(el) {
  let def;
  try {
    def = JSON.parse(decodeURIComponent(el.dataset.graph));
  } catch {
    el.classList.add('inline-graph--error');
    el.textContent = 'Could not parse graph definition.';
    return;
  }

  const { repo = '', version = '', symbols = [], relations = [] } = def;
  if (!symbols.length) return;

  const nodeEls = symbols.map(s => ({
    group: 'nodes',
    classes: `symbol-node kind-${(s.kind ?? 'unknown').toLowerCase()}`,
    data: {
      id: s.name,
      label: s.name,
      file: s.file ?? '',
      kind: s.kind ?? 'function',
      start_line: s.start_line ?? null,
      color: kindColor(s.kind),
      shape: kindShape(s.kind),
    },
  }));

  const edgeEls = relations
    .filter(r => r.source && r.target)
    .map((r, i) => ({
      group: 'edges',
      data: { id: `e${i}`, source: r.source, target: r.target, relation: r.relation },
    }));

  const cy = cytoscape({
    container: el,
    elements: [...nodeEls, ...edgeEls],
    style: cytoscapeStyle(),
    minZoom: 0.2,
    maxZoom: 3,
  });

  cy.layout({
    name: 'fcose',
    quality: 'default',
    randomize: false,
    nodeDimensionsIncludeLabels: true,
    nodeRepulsion: 14000,
    idealEdgeLength: 110,
    numIter: 600,
    gravity: 0.12,
    fit: true,
    padding: 24,
  }).run();

  const buildSections = node =>
    RELATION_DEFS.flatMap(({ rel, out, inc }) => {
      const targets = node.outgoers(`edge[relation="${rel}"]`).targets();
      const sources = node.incomers(`edge[relation="${rel}"]`).sources();
      const toSection = (label, nodes) => ({
        label,
        chips: nodes.map(n => ({
          name: n.data('label'),
          title: n.data('file'),
          onClick: () => tapNode(n),
        })),
      });
      return [
        ...(targets.length ? [toSection(out, targets)] : []),
        ...(sources.length ? [toSection(inc, sources)] : []),
      ];
    });

  function tapNode(node) {
    cy.elements().addClass('dimmed').removeClass('ring active').deselect();
    node.closedNeighborhood().removeClass('dimmed');
    node.addClass('ring');
    node.connectedEdges().addClass('active');
    node.select();
    openSourcePanel(node.data(), repo, version, buildSections(node));
  }

  cy.on('tap', 'node.symbol-node', evt => tapNode(evt.target));
  cy.on('tap', e => {
    if (e.target === cy) {
      cy.elements().removeClass('dimmed ring active').deselect();
      closeSourcePanel();
    }
  });
}

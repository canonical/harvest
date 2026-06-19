import { describe, it, expect, vi, beforeEach } from 'vitest';

// ── Cytoscape mock ────────────────────────────────────────────────────────────
// We need a controllable fake so we can simulate node/background taps without
// a real rendering engine. The factory captures the `on` calls so each test
// can retrieve the registered handlers by event + selector.

vi.mock('cytoscape-fcose', () => ({ default: {} }));

vi.mock('cytoscape', () => {
  function buildCyInstance() {
    const chainable = () => ({
      addClass:    vi.fn().mockReturnThis(),
      removeClass: vi.fn().mockReturnThis(),
      deselect:    vi.fn().mockReturnThis(),
    });
    return {
      elements: vi.fn(chainable),
      layout:   vi.fn(() => ({ run: vi.fn() })),
      on:       vi.fn(),
    };
  }
  const ctor = vi.fn(buildCyInstance);
  ctor.use = vi.fn();
  return { default: ctor };
});

import cytoscape from 'cytoscape';
import { mountInlineGraphs } from '../../src/lib/inline-graph.js';

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeElement(graphDef) {
  const el = document.createElement('div');
  el.className = 'inline-graph';
  el.dataset.graph = encodeURIComponent(JSON.stringify(graphDef));
  return el;
}

function makeContainer(graphDef) {
  const container = document.createElement('div');
  const el = makeElement(graphDef);
  container.appendChild(el);
  document.body.appendChild(container);
  return { container, el };
}

// Returns the handler registered for a given event + optional selector.
// inline-graph.js calls cy.on(event, selector, fn) for node taps and
// cy.on(event, fn) for background taps.
function getHandler(cyInstance, event, selector = null) {
  const call = cyInstance.on.mock.calls.find(([ev, selectorOrFn, fn]) => {
    if (ev !== event) return false;
    if (selector) return selectorOrFn === selector && typeof fn === 'function';
    return typeof selectorOrFn === 'function'; // background handler: no selector
  });
  if (!call) return null;
  return selector ? call[2] : call[1];
}

// Minimal fake cytoscape node that satisfies tapNode() and buildSections().
function fakeNode(overrides = {}) {
  const data = { name: 'MyFunc', kind: 'function', file: 'src/lib.rs', start_line: 5, label: 'MyFunc', ...overrides };
  const emptyCollection = { length: 0 };
  return {
    data:               vi.fn(() => data),
    addClass:           vi.fn().mockReturnThis(),
    select:             vi.fn(),
    closedNeighborhood: vi.fn(() => ({ removeClass: vi.fn().mockReturnThis() })),
    connectedEdges:     vi.fn(() => ({ addClass: vi.fn().mockReturnThis() })),
    outgoers:           vi.fn(() => ({ targets: vi.fn(() => emptyCollection) })),
    incomers:           vi.fn(() => ({ sources: vi.fn(() => emptyCollection) })),
  };
}

const SIMPLE_GRAPH = {
  repo:      'myrepo',
  version:   'v1.0',
  symbols:   [{ name: 'MyFunc', kind: 'function', file: 'src/lib.rs', start_line: 5 }],
  relations: [],
};

// ── Tests ─────────────────────────────────────────────────────────────────────

beforeEach(() => {
  vi.clearAllMocks();
  document.body.innerHTML = '';
});

describe('mountInlineGraphs — element lifecycle', () => {
  it('marks the element as mounted', () => {
    const { container, el } = makeContainer(SIMPLE_GRAPH);
    mountInlineGraphs(container);
    expect(el.classList.contains('inline-graph--mounted')).toBe(true);
  });

  it('skips elements already marked as mounted', () => {
    const { container, el } = makeContainer(SIMPLE_GRAPH);
    el.classList.add('inline-graph--mounted');
    mountInlineGraphs(container);
    expect(cytoscape).not.toHaveBeenCalled();
  });

  it('does not double-mount when called twice on the same container', () => {
    const { container } = makeContainer(SIMPLE_GRAPH);
    mountInlineGraphs(container);
    mountInlineGraphs(container);
    expect(cytoscape).toHaveBeenCalledOnce();
  });

  it('adds inline-graph--error class for malformed JSON and does not throw', () => {
    const container = document.createElement('div');
    const el = document.createElement('div');
    el.className = 'inline-graph';
    el.dataset.graph = '%ZZ invalid';
    container.appendChild(el);
    document.body.appendChild(container);

    expect(() => mountInlineGraphs(container)).not.toThrow();
    expect(el.classList.contains('inline-graph--error')).toBe(true);
    expect(cytoscape).not.toHaveBeenCalled();
  });

  it('returns early (no cytoscape) when symbols array is empty', () => {
    const { container, el } = makeContainer({ repo: 'r', version: 'v', symbols: [], relations: [] });
    mountInlineGraphs(container);
    expect(el.classList.contains('inline-graph--mounted')).toBe(true);
    expect(cytoscape).not.toHaveBeenCalled();
  });

  it('mounts multiple graphs in one container', () => {
    const container = document.createElement('div');
    container.appendChild(makeElement(SIMPLE_GRAPH));
    container.appendChild(makeElement(SIMPLE_GRAPH));
    document.body.appendChild(container);

    mountInlineGraphs(container);
    expect(cytoscape).toHaveBeenCalledTimes(2);
  });
});

describe('mountInlineGraphs — node tap → open-source-panel event', () => {
  it('dispatches open-source-panel with nodeData, repo, and version', () => {
    const { container } = makeContainer(SIMPLE_GRAPH);
    mountInlineGraphs(container);

    const cy = cytoscape.mock.results[0].value;
    const handler = getHandler(cy, 'tap', 'node.symbol-node');
    expect(handler).toBeTruthy();

    const events = [];
    window.addEventListener('open-source-panel', e => events.push(e.detail));

    const node = fakeNode();
    handler({ target: node });

    expect(events).toHaveLength(1);
    expect(events[0].repo).toBe('myrepo');
    expect(events[0].version).toBe('v1.0');
    expect(events[0].nodeData.label).toBe('MyFunc');
    expect(events[0].nodeData.kind).toBe('function');
  });

  it('includes sections array in the event detail', () => {
    const { container } = makeContainer(SIMPLE_GRAPH);
    mountInlineGraphs(container);

    const cy = cytoscape.mock.results[0].value;
    const handler = getHandler(cy, 'tap', 'node.symbol-node');

    const events = [];
    window.addEventListener('open-source-panel', e => events.push(e.detail));
    handler({ target: fakeNode() });

    expect(Array.isArray(events[0].sections)).toBe(true);
  });
});

describe('mountInlineGraphs — background tap → close-source-panel event', () => {
  it('dispatches close-source-panel when tap target is the cy instance itself', () => {
    const { container } = makeContainer(SIMPLE_GRAPH);
    mountInlineGraphs(container);

    const cy = cytoscape.mock.results[0].value;
    const handler = getHandler(cy, 'tap'); // no selector = background handler
    expect(handler).toBeTruthy();

    const fired = [];
    window.addEventListener('close-source-panel', () => fired.push(true));
    handler({ target: cy });

    expect(fired).toHaveLength(1);
  });

  it('does NOT dispatch close-source-panel when tap target is a node', () => {
    const { container } = makeContainer(SIMPLE_GRAPH);
    mountInlineGraphs(container);

    const cy = cytoscape.mock.results[0].value;
    const handler = getHandler(cy, 'tap');

    const fired = [];
    window.addEventListener('close-source-panel', () => fired.push(true));
    handler({ target: fakeNode() }); // target !== cy

    expect(fired).toHaveLength(0);
  });
});

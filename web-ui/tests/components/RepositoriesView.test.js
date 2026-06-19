import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import RepositoriesView from '../../src/views/RepositoriesView.vue';

// Cytoscape needs a real DOM canvas — stub it for jsdom
vi.mock('cytoscape', () => {
  const mockCy = vi.fn(() => ({
    elements: () => ({
      style: vi.fn(), animate: vi.fn(), addClass: vi.fn(), removeClass: vi.fn(),
      filter: vi.fn(() => ({ length: 0, closedNeighborhood: vi.fn(() => ({ forEach: vi.fn(), removeClass: vi.fn() })), forEach: vi.fn() })),
      forEach: vi.fn(),
    }),
    nodes: () => ({
      forEach: vi.fn(),
      filter: vi.fn(() => ({ has: vi.fn(() => false), closedNeighborhood: vi.fn(() => ({ removeClass: vi.fn(), forEach: vi.fn() })), forEach: vi.fn(), length: 0 })),
    }),
    on: vi.fn(), destroy: vi.fn(), resize: vi.fn(), fit: vi.fn(), animate: vi.fn(), zoom: vi.fn(() => 1),
  }));
  mockCy.use = vi.fn();
  return { default: mockCy };
});
vi.mock('cytoscape-fcose', () => ({ default: vi.fn() }));
vi.mock('../../src/lib/graph-utils.js', () => ({
  kindColor: vi.fn(() => '#000'),
  kindShape: vi.fn(() => 'rectangle'),
  cytoscapeStyle: vi.fn(() => []),
  KIND_COLORS: { class: '#7C3AED', struct: '#6D28D9', function: '#2563EB' },
}));

const REPOS = [
  { name: 'myrepo', versions: ['v1.0', 'v2.0'], url: 'https://github.com/org/myrepo' },
];

const GRAPH_DATA = {
  nodes: [
    { id: 'file:foo.rs:MyStruct', name: 'MyStruct', kind: 'Struct', file: 'foo.rs', start_line: 1 },
    { id: 'file:foo.rs:MyFn',    name: 'MyFn',     kind: 'Function', file: 'foo.rs', start_line: 10 },
  ],
  edges: [],
  truncated: false,
  total_nodes: 2,
};

function mockFetch(reposData = REPOS, graphData = GRAPH_DATA) {
  let call = 0;
  global.fetch = vi.fn(() => {
    const url = global.fetch.mock.calls[call]?.[0] ?? '';
    const result = url.includes('/graph') ? graphData : reposData;
    call++;
    return Promise.resolve({ ok: true, json: () => Promise.resolve(result) });
  });
}

describe('RepositoriesView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('renders repo select and version select', async () => {
    mockFetch();
    const w = mount(RepositoriesView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    const selects = w.findAll('select');
    expect(selects.length).toBeGreaterThanOrEqual(2);
  });

  it('populates repo options', async () => {
    mockFetch();
    const w = mount(RepositoriesView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    expect(w.text()).toContain('myrepo');
  });

  it('version select is disabled before repo is chosen', async () => {
    mockFetch();
    const w = mount(RepositoriesView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    const selects = w.findAll('select');
    expect(selects[1].attributes('disabled')).toBeDefined();
  });

  it('enables version select after repo chosen', async () => {
    mockFetch();
    const w = mount(RepositoriesView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    await w.findAll('select')[0].setValue('myrepo');
    await w.findAll('select')[0].trigger('change');
    await flushPromises();
    expect(w.findAll('select')[1].attributes('disabled')).toBeUndefined();
  });

  it('shows graph search input', async () => {
    mockFetch();
    const w = mount(RepositoriesView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    expect(w.find('input[type="search"], .graph-search').exists()).toBe(true);
  });

  it('shows cytoscape container element', async () => {
    mockFetch();
    const w = mount(RepositoriesView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    expect(w.find('#cy, .cy-container').exists()).toBe(true);
  });
});

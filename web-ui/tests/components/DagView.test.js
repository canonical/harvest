import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('cytoscape', () => {
  let tapHandler = null;
  const mockCy = {
    on(evt, selector, handler) {
      const fn = typeof selector === 'function' ? selector : handler;
      if (evt === 'tap') tapHandler = fn;
    },
    layout() { return { run() {} }; },
    elements() { return { remove() {} }; },
    add() {},
    destroy() {},
    nodes() { return { forEach() {} }; },
  };
  const mock = vi.fn(() => mockCy);
  mock.use = vi.fn();
  mock.__getTapHandler = () => tapHandler;
  mock.__reset = () => { tapHandler = null; };
  return { default: mock };
});
vi.mock('cytoscape-fcose', () => ({ default: vi.fn() }));

vi.mock('../../src/lib/graph-utils.js', () => ({
  kindColor: vi.fn(() => '#aaa'),
  kindShape: vi.fn(() => 'ellipse'),
  cytoscapeStyle: vi.fn(() => []),
}));

import DagView from '../../src/components/deployment/DagView.vue';

const PLAN = {
  deploy_steps: [
    {
      id: 's0', action: 'run', label: 'Prep', phase: 'deploy',
      artifact: { id: 'a0', kind: 'bash', title: 'Prep script' },
      depends_on: [],
    },
    {
      id: 's1', action: 'apply', label: 'Apply infra', phase: 'deploy',
      artifact: { id: 'a1', kind: 'terraform', title: 'Infra' },
      depends_on: ['s0'],
    },
  ],
  destroy_steps: [
    {
      id: 's2', action: 'destroy', label: 'Destroy infra', phase: 'destroy',
      artifact: { id: 'a1', kind: 'terraform', title: 'Infra' },
      depends_on: [],
    },
  ],
};

function mountDag(props = {}) {
  return mount(DagView, {
    props: {
      plan: PLAN,
      stepFiles: {},
      stepStatus: {},
      ...props,
    },
  });
}

describe('DagView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('renders a single canvas and deploy/destroy phase tabs', () => {
    const w = mountDag();
    expect(w.find('[data-testid="dag-canvas"]').exists()).toBe(true);
    const tabs = w.findAll('.dag-view__tab');
    expect(tabs).toHaveLength(2);
  });

  it('emits run-all when Run all button clicked', async () => {
    const w = mountDag();
    await w.find('[data-testid="run-all-btn"]').trigger('click');
    expect(w.emitted('run-all')).toBeTruthy();
  });

  it('emits select-artifact with the artifact id when a node is tapped', async () => {
    const cytoscape = (await import('cytoscape')).default;
    const w = mountDag();
    await flushPromises();
    const tapHandler = cytoscape.__getTapHandler();
    expect(tapHandler).toBeTruthy();
    tapHandler({ target: { data: () => ({ stepId: 's1', artifactId: 'a1' }) } });
    await flushPromises();
    expect(w.emitted('select-artifact')).toBeTruthy();
    expect(w.emitted('select-artifact')[0]).toEqual(['a1']);
  });

  it('does not emit select-artifact when the tapped node has no artifact', async () => {
    const cytoscape = (await import('cytoscape')).default;
    const w = mountDag();
    await flushPromises();
    const tapHandler = cytoscape.__getTapHandler();
    tapHandler({ target: { data: () => ({ stepId: 's1', artifactId: null }) } });
    await flushPromises();
    expect(w.emitted('select-artifact')).toBeFalsy();
  });

  it('shows per-step status from stepStatus prop', async () => {
    const w = mountDag({
      plan: PLAN,
      stepStatus: { s0: 'success', s1: 'failed' },
    });
    await w.vm.selectNode('s0');
    await flushPromises();
    expect(w.find('[data-testid="node-detail"]').text()).toContain('success');
  });

  it('closing the detail sidebar clears selection', async () => {
    const w = mountDag();
    await w.vm.selectNode('s1');
    await flushPromises();
    expect(w.find('[data-testid="node-detail"]').exists()).toBe(true);
    await w.find('.dag-view__detail-close').trigger('click');
    expect(w.find('[data-testid="node-detail"]').exists()).toBe(false);
  });
});

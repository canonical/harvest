import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('cytoscape', () => {
  const mockCy = {
    on: vi.fn(),
    layout: vi.fn(() => ({ run: vi.fn() })),
    elements: vi.fn(() => ({ remove: vi.fn() })),
    add: vi.fn(),
    destroy: vi.fn(),
  };
  const mock = vi.fn(() => mockCy);
  mock.use = vi.fn();
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

const STEP_FILES = { 'main.tf': 'resource "x" "y" {}' };

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

  it('clicking a node shows detail sidebar with file list for terraform', async () => {
    const w = mountDag({
      plan: PLAN,
      stepFiles: { s1: STEP_FILES },
    });
    await w.vm.selectNode('s1');
    await flushPromises();
    const detail = w.find('[data-testid="node-detail"]');
    expect(detail.exists()).toBe(true);
    expect(detail.text()).toContain('main.tf');
  });

  it('node detail shows plan preview button for terraform artifacts', async () => {
    const w = mountDag({
      plan: PLAN,
      stepFiles: { s1: STEP_FILES },
    });
    await w.vm.selectNode('s1');
    await flushPromises();
    expect(w.find('[data-testid="plan-preview-btn"]').exists()).toBe(true);
  });

  it('emits run-node for per-node run', async () => {
    const w = mountDag();
    await w.vm.selectNode('s1');
    await flushPromises();
    await w.find('[data-testid="run-node-btn"]').trigger('click');
    expect(w.emitted('run-node')).toBeTruthy();
    expect(w.emitted('run-node')[0][0]).toBe('s1');
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

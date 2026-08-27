import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getExecutionPlan: vi.fn(),
    runDag:           vi.fn(),
    getArtifact:      vi.fn(),
    openProjectEvents: vi.fn(() => ({ close() {} })),
  };
});

vi.mock('../../src/components/deployment/DagView.vue', () => ({
  default: {
    name: 'DagView',
    template: '<div data-testid="dag-view"><button data-testid="stub-node" @click="$emit(\'select-artifact\', \'a1\')" /><button data-testid="stub-run-all" @click="$emit(\'run-all\')" /></div>',
    props: ['plan', 'stepFiles', 'stepStatus'],
    emits: ['run-all', 'run-node', 'plan-preview', 'select-artifact'],
  },
}));
vi.mock('../../src/components/deployment/ArtifactEditor.vue', () => ({
  default: {
    name: 'ArtifactEditor',
    template: '<div data-testid="artifact-editor" />',
    props: ['projectId', 'deploymentId', 'artifactId'],
    emits: ['saved'],
  },
}));

import DeployArtifacts from '../../src/components/deployment/DeployArtifacts.vue';
import * as api from '../../src/lib/api.js';

const DEPLOYMENT = {
  id: 'd1', infra_state: 'none',
  terraform_bundle: { id: 'b1', kind: 'terraform' },
  context_artifacts: [],
};

const PLAN = {
  deploy_steps: [
    { id: 's0', action: 'run', label: 'Prep', artifact: { id: 'a0', kind: 'bash', title: 'Prep' }, depends_on: [] },
    { id: 's1', action: 'apply', label: 'Apply', artifact: { id: 'a1', kind: 'terraform', title: 'Infra' }, depends_on: ['s0'] },
  ],
  destroy_steps: [],
};

function mountPanel({ deployment = DEPLOYMENT, agents = [] } = {}) {
  return mount(DeployArtifacts, {
    props: { projectId: 'proj-1', deployment, agents },
  });
}

describe('DeployArtifacts', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.getExecutionPlan.mockResolvedValue(PLAN);
    api.getArtifact.mockResolvedValue({ id: 'a1', title: 'Infra', kind: 'terraform', content: '{}' });
  });

  it('renders the DAG on the left and editor on the right', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="dag-view"]').exists()).toBe(true);
    expect(w.find('[data-testid="artifact-editor"]').exists()).toBe(true);
  });

  it('loads the execution plan on mount', async () => {
    mountPanel();
    await flushPromises();
    expect(api.getExecutionPlan).toHaveBeenCalledWith('proj-1', 'd1');
  });

  it('passes the plan to DagView', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.findComponent({ name: 'DagView' }).props('plan')).toEqual(PLAN);
  });

  it('selects the corresponding artifact when a DAG node is clicked', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="stub-node"]').trigger('click');
    await flushPromises();
    expect(w.findComponent({ name: 'ArtifactEditor' }).props('artifactId')).toBe('a1');
  });

  it('passes projectId and deploymentId to the ArtifactEditor', async () => {
    const w = mountPanel();
    await flushPromises();
    const editor = w.findComponent({ name: 'ArtifactEditor' });
    expect(editor.props('projectId')).toBe('proj-1');
    expect(editor.props('deploymentId')).toBe('d1');
  });

  it('shows no artifact selected initially', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.findComponent({ name: 'ArtifactEditor' }).props('artifactId')).toBeNull();
  });

  it('shows an agent selector when agents are connected', async () => {
    const w = mountPanel({ agents: [{ id: 'ag-1', hostname: 'box1' }] });
    await flushPromises();
    expect(w.find('[data-testid="agent-select"]').exists()).toBe(true);
  });

  it('shows Run all button that emits run-dag', async () => {
    const w = mountPanel({ agents: [{ id: 'ag-1', hostname: 'box1' }] });
    await flushPromises();
    await w.find('[data-testid="run-all-btn"]').trigger('click');
    await flushPromises();
    expect(api.runDag).toHaveBeenCalledWith('proj-1', 'd1', { agent_id: 'ag-1', timeout_secs: 300 });
  });

  it('shows the infra-state badge', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('.infra-state-badge').exists()).toBe(true);
  });

  it('emits refresh when ArtifactEditor emits saved', async () => {
    const w = mountPanel();
    await flushPromises();
    w.findComponent({ name: 'ArtifactEditor' }).vm.$emit('saved');
    await flushPromises();
    expect(w.emitted('refresh')).toBeTruthy();
  });
});

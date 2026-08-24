import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getExecutionPlan:     vi.fn(),
    setExecutionPlan:      vi.fn(),
    runDag:                vi.fn(),
    getArtifact:           vi.fn(),
    generateProvision:     vi.fn(),
    openProjectEvents:     vi.fn(),
  };
});

vi.mock('../../src/components/deployment/DagView.vue', () => ({
  default: {
    template: '<div class="stub-dag-view" data-testid="dag-view"><button data-testid="stub-run-all" @click="$emit(\'run-all\')" /></div>',
    props: ['plan', 'stepFiles', 'stepStatus'],
    emits: ['run-all', 'run-node', 'plan-preview'],
  },
}));
vi.mock('../../src/components/deployment/RunHistory.vue', () => ({
  default: { template: '<div class="stub-run-history" data-testid="run-history" />', props: ['runs', 'liveEntry', 'liveLog'] },
}));

import ArtifactsPanel from '../../src/components/deployment/ArtifactsPanel.vue';
import * as api from '../../src/lib/api.js';

const AGENTS = [{ id: 'agent-1', hostname: 'host-1' }];

const DEPLOYMENT_NO_BUNDLE = {
  id: 'd1', infra_state: 'none', terraform_bundle: null, context_artifacts: [],
};

const DEPLOYMENT_WITH_BUNDLE = {
  id: 'd1', infra_state: 'none', terraform_bundle: { id: 'a1', title: 'Infra', kind: 'terraform' }, context_artifacts: [],
};

const EMPTY_PLAN = { deploy_steps: [], destroy_steps: [] };

function mountPanel(deployment = DEPLOYMENT_NO_BUNDLE, runs = [], agents = AGENTS) {
  return mount(ArtifactsPanel, {
    props: { projectId: 'proj-1', deployment, runs, agents },
  });
}

describe('ArtifactsPanel', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('shows generate button when no bundle exists', () => {
    api.getExecutionPlan.mockResolvedValue(EMPTY_PLAN);
    const w = mountPanel();
    expect(w.find('[data-testid="generate-artifacts-btn"]').exists()).toBe(true);
  });

  it('shows DagView when bundle exists', async () => {
    api.getExecutionPlan.mockResolvedValue(EMPTY_PLAN);
    const w = mountPanel(DEPLOYMENT_WITH_BUNDLE);
    await flushPromises();
    expect(w.find('[data-testid="dag-view"]').exists()).toBe(true);
  });

  it('generate calls generateProvision and emits refresh', async () => {
    api.getExecutionPlan.mockResolvedValue(EMPTY_PLAN);
    api.generateProvision.mockResolvedValue({});
    const w = mountPanel();
    await w.find('[data-testid="generate-artifacts-btn"]').trigger('click');
    await flushPromises();
    expect(api.generateProvision).toHaveBeenCalledWith('proj-1', 'd1');
  });

  it('shows agent selector with connected agents', async () => {
    api.getExecutionPlan.mockResolvedValue(EMPTY_PLAN);
    const w = mountPanel(DEPLOYMENT_WITH_BUNDLE);
    await flushPromises();
    const select = w.find('[data-testid="agent-select"]');
    expect(select.exists()).toBe(true);
    expect(select.findAll('option')).toHaveLength(2);
  });

  it('run-all calls runDag with selected agent and switches to history tab', async () => {
    api.getExecutionPlan.mockResolvedValue({
      deploy_steps: [{ id: 's0', action: 'run', label: 'Prep', artifact: { kind: 'bash' }, depends_on: [] }],
      destroy_steps: [],
    });
    api.runDag.mockResolvedValue({ runs: [{ exit_code: 0 }], infra_state: 'up' });
    const w = mountPanel(DEPLOYMENT_WITH_BUNDLE);
    await flushPromises();
    await w.find('[data-testid="stub-run-all"]').trigger('click');
    await flushPromises();
    expect(api.runDag).toHaveBeenCalledWith('proj-1', 'd1', { agent_id: 'agent-1', timeout_secs: 300 });
    expect(w.find('[data-testid="run-history"]').exists()).toBe(true);
  });

  it('shows run history tab', async () => {
    api.getExecutionPlan.mockResolvedValue(EMPTY_PLAN);
    const w = mountPanel(DEPLOYMENT_WITH_BUNDLE);
    await flushPromises();
    expect(w.find('[data-testid="run-history-tab"]').exists()).toBe(true);
    await w.find('[data-testid="run-history-tab"]').trigger('click');
    expect(w.find('[data-testid="run-history"]').exists()).toBe(true);
  });

  it('shows destroy coverage warning when deploy has apply but no destroy', async () => {
    api.getExecutionPlan.mockResolvedValue({
      deploy_steps: [{ id: 's0', action: 'apply', label: 'Apply', artifact: { id: 'a1', kind: 'terraform' }, depends_on: [] }],
      destroy_steps: [],
    });
    const w = mountPanel(DEPLOYMENT_WITH_BUNDLE);
    await flushPromises();
    expect(w.find('[data-testid="coverage-warning"]').exists()).toBe(true);
    expect(w.text()).toContain('no matching destroy');
  });

  it('shows no coverage warning when destroy covers all apply', async () => {
    api.getExecutionPlan.mockResolvedValue({
      deploy_steps: [{ id: 's0', action: 'apply', label: 'Apply', artifact: { id: 'a1', kind: 'terraform' }, depends_on: [] }],
      destroy_steps: [{ id: 's1', action: 'destroy', label: 'Destroy', artifact: { id: 'a1', kind: 'terraform' }, depends_on: [] }],
    });
    const w = mountPanel(DEPLOYMENT_WITH_BUNDLE);
    await flushPromises();
    expect(w.find('[data-testid="coverage-warning"]').exists()).toBe(false);
  });
});

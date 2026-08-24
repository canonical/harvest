import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getProjectDeploymentSingle: vi.fn(),
    listDeploymentRuns:       vi.fn(),
    listProjectAgents:         vi.fn(),
    listDeploymentProposals:   vi.fn(),
  };
});

import DeployView from '../../src/views/DeployView.vue';
import * as api from '../../src/lib/api.js';

const DEPLOYMENT = {
  id: 'd1', name: 'MyProject', infra_state: 'none', terraform_bundle: null,
};

function mountView() {
  return mount(DeployView, { props: { projectId: 'proj-1' } });
}

describe('DeployView', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT));
    api.listDeploymentRuns.mockResolvedValue([]);
    api.listProjectAgents.mockResolvedValue([]);
    api.listDeploymentProposals.mockResolvedValue([]);
  });

  it('fetches the project deployment on mount', async () => {
    const w = mountView();
    await flushPromises();
    expect(api.getProjectDeploymentSingle).toHaveBeenCalledWith('proj-1');
  });

  it('shows the infra-state badge', async () => {
    const d = { ...DEPLOYMENT, infra_state: 'up' };
    api.getProjectDeploymentSingle.mockResolvedValue(d);
    const w = mountView();
    await flushPromises();
    expect(w.find('.infra-state-badge').exists()).toBe(true);
  });

  it('renders the ArtifactsPanel with the fetched deployment', async () => {
    const w = mountView();
    await flushPromises();
    expect(w.findComponent({ name: 'ArtifactsPanel' }).exists()).toBe(true);
  });

  it('renders the RunHistory component', async () => {
    const w = mountView();
    await flushPromises();
    expect(w.findComponent({ name: 'RunHistory' }).exists()).toBe(true);
  });
});

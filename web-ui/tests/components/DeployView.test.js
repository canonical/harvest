import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { createRouter, createMemoryHistory } from 'vue-router';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getProjectDeploymentSingle: vi.fn(),
    listProjectAgents:         vi.fn(),
    openProjectEvents:          vi.fn(() => ({ close() {} })),
    listDeploymentProposals:   vi.fn(),
    generateProvisionStream:   vi.fn(() => new Promise(() => {})),
  };
});

vi.mock('../../src/components/deployment/DeployArtifacts.vue', () => ({
  default: {
    name: 'DeployArtifacts',
    template: '<div data-testid="artifacts-panel" />',
    props: ['projectId', 'deployment', 'agents'],
    emits: ['refresh'],
  },
}));
vi.mock('../../src/components/deployment/DeployAgentsPanel.vue', () => ({
  default: {
    name: 'DeployAgentsPanel',
    template: '<div data-testid="deploy-agents-panel"><button data-testid="stub-next" @click="$emit(\'next\')" /></div>',
    props: ['projectId', 'agents', 'reload'],
    emits: ['next'],
  },
}));
vi.mock('../../src/components/deployment/DeployGenerationPanel.vue', () => ({
  default: {
    name: 'DeployGenerationPanel',
    template: '<div data-testid="deploy-generation-panel" />',
    props: ['projectId', 'deploymentId', 'deploymentName'],
    emits: ['done'],
  },
}));

import DeployView from '../../src/views/DeployView.vue';
import * as api from '../../src/lib/api.js';

const DEPLOYMENT_NO_DESIGN = {
  id: 'd1', name: 'MyProject', infra_state: 'none', design_doc: null, terraform_bundle: null,
};

const DEPLOYMENT_WITH_DESIGN = {
  id: 'd1', name: 'MyProject', infra_state: 'none',
  design_doc: { id: 'a1', title: 'Design doc' }, terraform_bundle: null,
};

const DEPLOYMENT_WITH_BUNDLE = {
  id: 'd1', name: 'MyProject', infra_state: 'up',
  design_doc: { id: 'a1', title: 'Design doc' },
  terraform_bundle: { id: 'b1', kind: 'terraform' },
};

const AGENTS = [{ id: 'ag-1', hostname: 'box1', online: true, last_seen: new Date().toISOString() }];

let pinia;
function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/design', component: { template: '<div />' } },
    ],
  });
}

async function mountView() {
  const router = makeRouter();
  return mount(DeployView, {
    props: { projectId: 'proj-1' },
    global: { plugins: [pinia, router] },
  });
}

describe('DeployView', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    pinia = createPinia();
    setActivePinia(pinia);
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_NO_DESIGN));
    api.listProjectAgents.mockResolvedValue(structuredClone(AGENTS));
    api.listDeploymentProposals.mockResolvedValue([]);
  });

  it('fetches the project deployment on mount', async () => {
    await mountView();
    await flushPromises();
    expect(api.getProjectDeploymentSingle).toHaveBeenCalledWith('proj-1');
  });

  it('shows a loading indicator while fetching', async () => {
    let resolveFn;
    api.getProjectDeploymentSingle.mockReturnValue(new Promise(r => { resolveFn = r; }));
    const w = await mountView();
    await flushPromises();
    expect(w.find('[data-testid="deploy-loading"]').exists()).toBe(true);
    resolveFn(structuredClone(DEPLOYMENT_NO_DESIGN));
    await flushPromises();
  });

  it('asks for a design first with a link to the Design page when there is no design_doc', async () => {
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_NO_DESIGN));
    const w = await mountView();
    await flushPromises();
    expect(w.find('[data-testid="deploy-needs-design"]').exists()).toBe(true);
    const link = w.find('[data-testid="deploy-go-to-design"]');
    expect(link.exists()).toBe(true);
    expect(link.attributes('href')).toBe('/design');
  });

  it('renders the DeployAgentsPanel when a design exists but no terraform bundle yet', async () => {
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_WITH_DESIGN));
    const w = await mountView();
    await flushPromises();
    expect(w.find('[data-testid="deploy-agents-panel"]').exists()).toBe(true);
    expect(w.find('[data-testid="artifacts-panel"]').exists()).toBe(false);
  });

  it('passes the connected agents to the DeployAgentsPanel', async () => {
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_WITH_DESIGN));
    api.listProjectAgents.mockResolvedValue(structuredClone(AGENTS));
    const w = await mountView();
    await flushPromises();
    expect(w.findComponent({ name: 'DeployAgentsPanel' }).props('agents')).toEqual(AGENTS);
  });

  it('renders the DeployArtifacts panel when a terraform bundle exists', async () => {
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_WITH_BUNDLE));
    const w = await mountView();
    await flushPromises();
    expect(w.find('[data-testid="artifacts-panel"]').exists()).toBe(true);
    expect(w.find('[data-testid="deploy-agents-panel"]').exists()).toBe(false);
  });

  it('shows the broken banner when infra_state is broken', async () => {
    api.getProjectDeploymentSingle.mockResolvedValue({ ...structuredClone(DEPLOYMENT_WITH_BUNDLE), infra_state: 'broken' });
    const w = await mountView();
    await flushPromises();
    expect(w.find('[data-testid="broken-banner"]').exists()).toBe(true);
  });

  it('moves to the generating state when DeployAgentsPanel emits next', async () => {
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_WITH_DESIGN));
    const w = await mountView();
    await flushPromises();
    await w.find('[data-testid="stub-next"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="deploy-generation-panel"]').exists()).toBe(true);
    expect(w.find('[data-testid="deploy-agents-panel"]').exists()).toBe(false);
  });

  it('passes projectId and deployment id to the DeployGenerationPanel', async () => {
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_WITH_DESIGN));
    const w = await mountView();
    await flushPromises();
    await w.find('[data-testid="stub-next"]').trigger('click');
    await flushPromises();
    const gen = w.findComponent({ name: 'DeployGenerationPanel' });
    expect(gen.props('projectId')).toBe('proj-1');
    expect(gen.props('deploymentId')).toBe('d1');
    expect(gen.props('deploymentName')).toBe('MyProject');
  });

  it('refetches deployment and shows ArtifactsPanel when generation finishes with a bundle', async () => {
    api.getProjectDeploymentSingle
      .mockResolvedValueOnce(structuredClone(DEPLOYMENT_WITH_DESIGN))
      .mockResolvedValueOnce(structuredClone(DEPLOYMENT_WITH_BUNDLE));
    const w = await mountView();
    await flushPromises();
    await w.find('[data-testid="stub-next"]').trigger('click');
    await flushPromises();
    w.findComponent({ name: 'DeployGenerationPanel' }).vm.$emit('done');
    await flushPromises();
    expect(api.getProjectDeploymentSingle).toHaveBeenCalledTimes(2);
    expect(w.find('[data-testid="artifacts-panel"]').exists()).toBe(true);
    expect(w.find('[data-testid="deploy-generation-panel"]').exists()).toBe(false);
  });

  it('falls back to DeployAgentsPanel when generation finishes without a bundle', async () => {
    api.getProjectDeploymentSingle
      .mockResolvedValueOnce(structuredClone(DEPLOYMENT_WITH_DESIGN))
      .mockResolvedValueOnce(structuredClone(DEPLOYMENT_WITH_DESIGN));
    const w = await mountView();
    await flushPromises();
    await w.find('[data-testid="stub-next"]').trigger('click');
    await flushPromises();
    w.findComponent({ name: 'DeployGenerationPanel' }).vm.$emit('done');
    await flushPromises();
    expect(w.find('[data-testid="deploy-agents-panel"]').exists()).toBe(true);
    expect(w.find('[data-testid="deploy-generation-panel"]').exists()).toBe(false);
  });
});

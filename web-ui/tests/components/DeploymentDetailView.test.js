import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createRouter, createMemoryHistory } from 'vue-router';

const DEPLOYMENT_FRESH = {
  id: 'd1', name: 'Acme rollout', environment_description: '',
  infra_state: 'none', template: null,
  design_doc: null, terraform_bundle: null, guide: null,
  created_at: new Date().toISOString(), updated_at: new Date().toISOString(),
};

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getProjectDeployment: vi.fn(),
    listDeploymentRuns:   vi.fn(),
    listProjectAgents:    vi.fn(),
  };
});

vi.mock('../../src/components/deployment/EnvironmentPhase.vue', () => ({
  default: { template: '<div class="stub-environment-phase" />', props: ['projectId', 'deployment'] },
}));
vi.mock('../../src/components/deployment/DesignPhase.vue', () => ({
  default: { template: '<div class="stub-design-phase" />', props: ['projectId', 'deployment'] },
}));
vi.mock('../../src/components/deployment/ProvisionPhase.vue', () => ({
  default: { template: '<div class="stub-provision-phase" />', props: ['projectId', 'deployment', 'runs', 'agents'] },
}));

import DeploymentDetailView from '../../src/views/DeploymentDetailView.vue';
import * as api from '../../src/lib/api.js';

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/deployments/:id', component: DeploymentDetailView }],
  });
}

async function mountView({ deployment = DEPLOYMENT_FRESH, runs = [] } = {}) {
  api.getProjectDeployment.mockResolvedValue(deployment);
  api.listDeploymentRuns.mockResolvedValue(runs);
  api.listProjectAgents.mockResolvedValue([]);

  const router = makeRouter();
  router.push('/deployments/d1');
  await router.isReady();
  const w = mount(DeploymentDetailView, {
    props: { projectId: 'proj-1' },
    global: { plugins: [router] },
  });
  await flushPromises();
  return { w, router };
}

describe('DeploymentDetailView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('loads the deployment and shows its name and infra state', async () => {
    const { w } = await mountView();
    expect(w.text()).toContain('Acme rollout');
    expect(w.text()).toContain('Not deployed');
  });

  it('renders all five phase tabs', async () => {
    const { w } = await mountView();
    const text = w.text();
    for (const label of ['Describe environment', 'Design', 'Deploy', 'Validate', 'Guide']) {
      expect(text).toContain(label);
    }
  });

  it('defaults to the environment phase for a fresh deployment and shows its component', async () => {
    const { w } = await mountView();
    expect(w.find('.stub-environment-phase').exists()).toBe(true);
    expect(w.find('.stub-design-phase').exists()).toBe(false);
  });

  it('defaults to the design phase once the environment description is filled in', async () => {
    const { w } = await mountView({ deployment: { ...DEPLOYMENT_FRESH, environment_description: 'notes' } });
    expect(w.find('.stub-design-phase').exists()).toBe(true);
  });

  it('defaults to the provision phase once a design doc exists', async () => {
    const { w } = await mountView({
      deployment: { ...DEPLOYMENT_FRESH, environment_description: 'notes', design_doc: { id: 'a1', title: 'Design' } },
    });
    expect(w.find('.stub-provision-phase').exists()).toBe(true);
  });

  it('clicking a tab switches the visible phase component', async () => {
    const { w } = await mountView({ deployment: { ...DEPLOYMENT_FRESH, environment_description: 'notes', design_doc: { id: 'a1' } } });
    expect(w.find('.stub-provision-phase').exists()).toBe(true);

    const tabs = w.findAll('.deployment-phase-tab');
    const envTab = tabs.find(t => t.text().includes('Describe environment'));
    await envTab.trigger('click');
    expect(w.find('.stub-environment-phase').exists()).toBe(true);
  });

  it('disables the Validate and Guide tabs', async () => {
    const { w } = await mountView();
    const tabs = w.findAll('.deployment-phase-tab');
    const validateTab = tabs.find(t => t.text().includes('Validate'));
    const guideTab = tabs.find(t => t.text().includes('Guide'));
    expect(validateTab.attributes('disabled')).toBeDefined();
    expect(guideTab.attributes('disabled')).toBeDefined();
  });

  it('marks a phase tab done once its condition is met', async () => {
    const { w } = await mountView({ deployment: { ...DEPLOYMENT_FRESH, environment_description: 'notes' } });
    const tabs = w.findAll('.deployment-phase-tab');
    const envTab = tabs.find(t => t.text().includes('Describe environment'));
    expect(envTab.classes()).toContain('deployment-phase-tab--done');
  });
});

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createRouter, createMemoryHistory } from 'vue-router';

const DEPLOYMENT_FRESH = {
  id: 'd1', name: 'Acme rollout', environment_description: '',
  infra_state: 'none', template: null,
  design_doc: null, terraform_bundle: null, guide: null,
  context_artifacts: [],
  created_at: new Date().toISOString(), updated_at: new Date().toISOString(),
};

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getProjectDeployment:     vi.fn(),
    listDeploymentRuns:       vi.fn(),
    listProjectAgents:        vi.fn(),
    listDeploymentProposals:  vi.fn(),
  };
});

vi.mock('../../src/components/deployment/ContextArtifactsPanel.vue', () => ({
  default: { template: '<div data-testid="context-panel" />', props: ['projectId', 'deployment'] },
}));
vi.mock('../../src/components/deployment/DesignPanel.vue', () => ({
  default: { template: '<div data-testid="design-panel" />', props: ['projectId', 'deployment'] },
}));
vi.mock('../../src/components/deployment/ArtifactsPanel.vue', () => ({
  default: { template: '<div data-testid="artifacts-panel" />', props: ['projectId', 'deployment', 'runs', 'agents'] },
}));
vi.mock('../../src/components/deployment/GuidePanel.vue', () => ({
  default: { template: '<div data-testid="guide-panel" />', props: ['projectId', 'deployment'] },
}));
vi.mock('../../src/components/deployment/ReviewInbox.vue', () => ({
  default: { template: '<div data-testid="review-inbox" />', props: ['projectId', 'deploymentId'] },
}));

import DeploymentDetailView from '../../src/views/DeploymentDetailView.vue';
import * as api from '../../src/lib/api.js';

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/deployments/:id', component: DeploymentDetailView }],
  });
}

async function mountView({ deployment = DEPLOYMENT_FRESH, runs = [], proposals = [] } = {}) {
  api.getProjectDeployment.mockResolvedValue(deployment);
  api.listDeploymentRuns.mockResolvedValue(runs);
  api.listProjectAgents.mockResolvedValue([]);
  api.listDeploymentProposals.mockResolvedValue(proposals);

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

  it('renders top tabs for all four sections', async () => {
    const { w } = await mountView();
    const tabs = w.findAll('.deployment-tab');
    const labels = tabs.map(t => t.text());
    expect(labels.some(l => l.includes('Context'))).toBe(true);
    expect(labels.some(l => l.includes('Design'))).toBe(true);
    expect(labels.some(l => l.includes('Artifacts'))).toBe(true);
    expect(labels.some(l => l.includes('Guide'))).toBe(true);
  });

  it('does not render the old phase tab bar or card rail', async () => {
    const { w } = await mountView();
    expect(w.find('.deployment-phase-tabs').exists()).toBe(false);
    expect(w.find('.deployment-card-rail').exists()).toBe(false);
  });

  it('shows context artifact count on the context tab', async () => {
    const { w } = await mountView({
      deployment: { ...DEPLOYMENT_FRESH, context_artifacts: [
        { id: 'ca1', title: 'Notes', kind: 'markdown' },
        { id: 'ca2', title: 'Diagram', kind: 'markdown' },
      ] },
    });
    const contextTab = w.findAll('.deployment-tab').find(t => t.text().includes('Context'));
    expect(contextTab.text()).toContain('2');
  });

  it('shows review button with count when pending proposals exist', async () => {
    const { w } = await mountView({
      proposals: [{ id: 'p1', status: 'pending', explanation: 'fix', target_artifact_kind: 'terraform' }],
    });
    const btn = w.find('[data-testid="review-toggle"]');
    expect(btn.exists()).toBe(true);
    expect(btn.text()).toContain('1');
  });

  it('clicking review button opens the review drawer', async () => {
    const { w } = await mountView({
      proposals: [{ id: 'p1', status: 'pending', explanation: 'fix', target_artifact_kind: 'terraform' }],
    });
    await w.find('[data-testid="review-toggle"]').trigger('click');
    expect(w.find('[data-testid="review-drawer"]').exists()).toBe(true);
  });

  it('selecting the context tab shows the context panel', async () => {
    const { w } = await mountView();
    w.findAll('.deployment-tab').find(t => t.text().includes('Context')).trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="context-panel"]').exists()).toBe(true);
  });

  it('selecting the design tab shows the design panel', async () => {
    const { w } = await mountView();
    w.findAll('.deployment-tab').find(t => t.text().includes('Design')).trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="design-panel"]').exists()).toBe(true);
  });

  it('selecting the artifacts tab shows the artifacts panel', async () => {
    const { w } = await mountView();
    w.findAll('.deployment-tab').find(t => t.text().includes('Artifacts')).trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="artifacts-panel"]').exists()).toBe(true);
  });

  it('shows broken badge and issue link when infra is broken', async () => {
    const { w } = await mountView({
      deployment: { ...DEPLOYMENT_FRESH, infra_state: 'broken' },
    });
    expect(w.text()).toContain('Broken');
    expect(w.find('[data-testid="view-issues-link"]').exists()).toBe(true);
  });
});

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { createRouter, createMemoryHistory } from 'vue-router';

const DEPLOYMENTS = [
  {
    id: 'd1', name: 'Acme rollout', infra_state: 'up', template: { id: 't1', name: 'Acme Gateway v3' },
    environment_description: 'env', updated_at: new Date('2026-01-02').toISOString(),
  },
  {
    id: 'd2', name: 'From-scratch rollout', infra_state: 'none', template: null,
    environment_description: 'env', updated_at: new Date('2026-01-01').toISOString(),
  },
];

const TEMPLATES = [{ id: 't1', name: 'Acme Gateway v3', description: '' }];

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    listProjectDeployments:  vi.fn(),
    createProjectDeployment: vi.fn(),
    listGroupTemplates:      vi.fn(),
  };
});

import DeploymentsView from '../../src/views/DeploymentsView.vue';
import { useProjectStore } from '../../src/stores/project.js';
import * as api from '../../src/lib/api.js';

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/deployments', component: DeploymentsView },
      { path: '/deployments/:id', component: { template: '<div>detail</div>' } },
    ],
  });
}

async function mountView({ projectId = 'proj-1', groupId = 'grp-1' } = {}) {
  setActivePinia(createPinia());
  const project = useProjectStore();
  if (groupId) project.selectProject({ id: projectId, name: 'Test project', group_id: groupId });

  const router = makeRouter();
  router.push('/deployments');
  await router.isReady();
  const w = mount(DeploymentsView, {
    props: { projectId },
    global: { plugins: [router] },
  });
  await flushPromises();
  return { w, router };
}

describe('DeploymentsView', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    api.listProjectDeployments.mockResolvedValue(DEPLOYMENTS);
    api.listGroupTemplates.mockResolvedValue(TEMPLATES);
    api.createProjectDeployment.mockResolvedValue({ id: 'd3' });
  });

  it('shows a no-project state when there is no projectId', async () => {
    const { w } = await mountView({ projectId: null });
    expect(w.text()).toContain('Select a project');
  });

  it('lists deployments with infra_state badges and template names', async () => {
    const { w } = await mountView();
    expect(w.text()).toContain('Acme rollout');
    expect(w.text()).toContain('Acme Gateway v3');
    expect(w.text()).toContain('From-scratch rollout');
    expect(w.text()).toContain('From scratch');
  });

  it('shows an empty state when there are no deployments', async () => {
    api.listProjectDeployments.mockResolvedValue([]);
    const { w } = await mountView();
    expect(w.text()).toContain('No deployments yet');
  });

  it('clicking a deployment row navigates to its detail page', async () => {
    const { w, router } = await mountView();
    await w.find('[data-testid="deployment-row-d1"]').trigger('click');
    await flushPromises();
    expect(router.currentRoute.value.path).toBe('/deployments/d1');
  });

  it('opening the new-deployment modal loads group templates into the select', async () => {
    const { w } = await mountView();
    await w.find('.new-deployment-btn').trigger('click');
    await flushPromises();
    expect(api.listGroupTemplates).toHaveBeenCalledWith('grp-1');
    const options = w.findAll('#deployment-template option').map(o => o.text());
    expect(options).toContain('Start from scratch');
    expect(options).toContain('Acme Gateway v3');
  });

  it('submitting the new-deployment form creates a deployment and navigates to it', async () => {
    const { w, router } = await mountView();
    await w.find('.new-deployment-btn').trigger('click');
    await flushPromises();

    await w.find('#deployment-name').setValue('New Customer');
    await w.find('#deployment-env').setValue('3 racks, air-gapped');
    await w.find('.modal-actions .p-button--positive').trigger('click');
    await flushPromises();

    expect(api.createProjectDeployment).toHaveBeenCalledWith('proj-1', {
      name: 'New Customer', environment_description: '3 racks, air-gapped', product_template_id: null,
    });
    expect(router.currentRoute.value.path).toBe('/deployments/d3');
  });

  it('the create button is disabled until a name is entered', async () => {
    const { w } = await mountView();
    await w.find('.new-deployment-btn').trigger('click');
    await flushPromises();
    expect(w.find('.modal-actions .p-button--positive').attributes('disabled')).toBeDefined();
  });
});

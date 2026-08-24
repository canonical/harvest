import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createRouter, createMemoryHistory } from 'vue-router';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    listChangeRequests:   vi.fn(),
    discardChangeRequest: vi.fn(),
  };
});

import ChangeRequestsView from '../../src/views/ChangeRequestsView.vue';
import * as api from '../../src/lib/api.js';

const CHANGE_REQUESTS = [
  { id: 'cr1', title: 'Apply fails on security group', status: 'open',      kind: 'issue',    deployment: { id: 'd1', name: 'Acme rollout' } },
  { id: 'cr2', title: 'Fix the design',                status: 'open',      kind: 'proposal', deployment: { id: 'd1', name: 'Acme rollout' } },
  { id: 'cr3', title: 'Timeout reaching health check', status: 'in_review', kind: 'issue',    deployment: { id: 'd1', name: 'Acme rollout' } },
  { id: 'cr4', title: 'DNS record missing',            status: 'applied',   kind: 'issue',    deployment: { id: 'd2', name: 'Beta rollout' } },
  { id: 'cr5', title: 'Wrong region',                  status: 'discarded', kind: 'proposal', deployment: { id: 'd2', name: 'Beta rollout' } },
];

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/change-requests', component: ChangeRequestsView },
      { path: '/change-requests/:id', component: { template: '<div />' } },
    ],
  });
}

async function mountView(crs = CHANGE_REQUESTS) {
  api.listChangeRequests.mockResolvedValue(structuredClone(crs));
  const router = makeRouter();
  router.push('/change-requests');
  await router.isReady();
  const w = mount(ChangeRequestsView, {
    props: { projectId: 'proj-1' },
    global: { plugins: [router] },
  });
  await flushPromises();
  return w;
}

describe('ChangeRequestsView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('renders a board with four columns', async () => {
    const w = await mountView();
    expect(w.find('[data-testid="change-requests-board"]').exists()).toBe(true);
    expect(w.find('[data-testid="issues-column-open"]').exists()).toBe(true);
    expect(w.find('[data-testid="issues-column-in_review"]').exists()).toBe(true);
    expect(w.find('[data-testid="issues-column-applied"]').exists()).toBe(true);
    expect(w.find('[data-testid="issues-column-discarded"]').exists()).toBe(true);
  });

  it('fetches change requests from the unified API', async () => {
    await mountView();
    expect(api.listChangeRequests).toHaveBeenCalledWith('proj-1', expect.anything());
  });

  it('places cards in the correct column by status', async () => {
    const w = await mountView();
    const openCol = w.find('[data-testid="issues-column-open"]');
    expect(openCol.findAll('[data-testid="issue-card"]').length).toBe(2);
  });

  it('shows the kind badge on each card', async () => {
    const w = await mountView();
    const cards = w.findAll('[data-testid="issue-card"]');
    expect(cards[0].text()).toContain('issue');
    expect(cards[1].text()).toContain('proposal');
  });

  it('navigates to detail on card click', async () => {
    const w = await mountView();
    const card = w.find('[data-testid="issue-card"]');
    await card.trigger('click');
    await w.vm.$router.isReady();
    await flushPromises();
    expect(w.vm.$router.currentRoute.value.path).toContain('/change-requests/cr1');
  });

  it('discards a change request via the discard button', async () => {
    api.discardChangeRequest.mockResolvedValue({});
    const w = await mountView();
    const discardBtn = w.find('[data-testid="move-issue-cr1-discarded"]');
    await discardBtn.trigger('click');
    await flushPromises();
    expect(api.discardChangeRequest).toHaveBeenCalledWith('proj-1', 'cr1');
  });
});

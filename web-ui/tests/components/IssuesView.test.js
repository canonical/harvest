import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createRouter, createMemoryHistory } from 'vue-router';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    listProjectIssues:  vi.fn(),
    updateIssueStatus:  vi.fn(),
  };
});

import IssuesView from '../../src/views/IssuesView.vue';
import * as api from '../../src/lib/api.js';

const ISSUES = [
  { id: 'i1', title: 'Apply fails on security group', status: 'untriaged', has_proposed_solution: false, deployment: { id: 'd1', name: 'Acme rollout' } },
  { id: 'i2', title: 'Timeout reaching health check', status: 'in_progress', has_proposed_solution: true, deployment: { id: 'd1', name: 'Acme rollout' } },
  { id: 'i3', title: 'DNS record missing', status: 'fixed', has_proposed_solution: false, deployment: { id: 'd2', name: 'Beta rollout' } },
  { id: 'i4', title: 'Wrong region', status: 'rejected', has_proposed_solution: false, deployment: { id: 'd2', name: 'Beta rollout' } },
];

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/issues', component: IssuesView },
      { path: '/issues/:id', component: { template: '<div />' } },
    ],
  });
}

async function mountView(issues = ISSUES) {
  api.listProjectIssues.mockResolvedValue(structuredClone(issues));
  const router = makeRouter();
  router.push('/issues');
  await router.isReady();
  const w = mount(IssuesView, {
    props: { projectId: 'proj-1' },
    global: { plugins: [router] },
  });
  await flushPromises();
  return { w, router };
}

describe('IssuesView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('shows a prompt when no project is selected', async () => {
    const router = makeRouter();
    router.push('/issues');
    await router.isReady();
    const w = mount(IssuesView, { props: { projectId: null }, global: { plugins: [router] } });
    expect(w.text()).toContain('Select a project');
  });

  it('renders four columns', async () => {
    const { w } = await mountView();
    for (const id of ['untriaged', 'in_progress', 'fixed', 'rejected']) {
      expect(w.find(`[data-testid="issues-column-${id}"]`).exists()).toBe(true);
    }
  });

  it('places each issue in the column matching its status', async () => {
    const { w } = await mountView();
    const untriaged = w.find('[data-testid="issues-column-untriaged"]');
    expect(untriaged.text()).toContain('Apply fails on security group');
    const inProgress = w.find('[data-testid="issues-column-in_progress"]');
    expect(inProgress.text()).toContain('Timeout reaching health check');
    const fixed = w.find('[data-testid="issues-column-fixed"]');
    expect(fixed.text()).toContain('DNS record missing');
    const rejected = w.find('[data-testid="issues-column-rejected"]');
    expect(rejected.text()).toContain('Wrong region');
  });

  it('shows a fix-proposed badge only on issues with a proposed solution', async () => {
    const { w } = await mountView();
    const cards = w.findAll('[data-testid="issue-card"]');
    const withFix = cards.find(c => c.text().includes('Timeout reaching health check'));
    const withoutFix = cards.find(c => c.text().includes('Apply fails on security group'));
    expect(withFix.text()).toContain('Fix proposed');
    expect(withoutFix.text()).not.toContain('Fix proposed');
  });

  it('offers only valid move buttons for an untriaged issue', async () => {
    const { w } = await mountView();
    expect(w.find('[data-testid="move-issue-i1-in_progress"]').exists()).toBe(true);
    expect(w.find('[data-testid="move-issue-i1-fixed"]').exists()).toBe(false);
    expect(w.find('[data-testid="move-issue-i1-rejected"]').exists()).toBe(false);
  });

  it('offers fixed and rejected moves for an in_progress issue, and none for terminal issues', async () => {
    const { w } = await mountView();
    expect(w.find('[data-testid="move-issue-i2-fixed"]').exists()).toBe(true);
    expect(w.find('[data-testid="move-issue-i2-rejected"]').exists()).toBe(true);
    expect(w.find('[data-testid="move-issue-i3-in_progress"]').exists()).toBe(false);
    expect(w.find('[data-testid="move-issue-i4-in_progress"]').exists()).toBe(false);
  });

  it('clicking a valid move button calls updateIssueStatus and moves the card', async () => {
    api.updateIssueStatus.mockResolvedValue({});
    const { w } = await mountView();
    await w.find('[data-testid="move-issue-i1-in_progress"]').trigger('click');
    await flushPromises();

    expect(api.updateIssueStatus).toHaveBeenCalledWith('proj-1', 'i1', 'in_progress');
    expect(w.find('[data-testid="issues-column-in_progress"]').text()).toContain('Apply fails on security group');
    expect(w.find('[data-testid="issues-column-untriaged"]').text()).not.toContain('Apply fails on security group');
  });

  it('reverts the optimistic move and shows an error when the server rejects it', async () => {
    api.updateIssueStatus.mockRejectedValue(new Error('cannot move an issue from untriaged to in_progress'));
    const { w } = await mountView();
    await w.find('[data-testid="move-issue-i1-in_progress"]').trigger('click');
    await flushPromises();

    expect(w.find('[data-testid="issues-column-untriaged"]').text()).toContain('Apply fails on security group');
    expect(w.find('[data-testid="issues-column-in_progress"]').text()).not.toContain('Apply fails on security group');
    expect(w.text()).toContain('cannot move an issue from untriaged to in_progress');
  });

  it('marks the drop target column disabled while dragging a card to an invalid column', async () => {
    const { w } = await mountView();
    const card = w.find('[data-testid="issue-card"]');
    await card.trigger('dragstart');

    const fixedColumn = w.find('[data-testid="issues-column-fixed"]');
    expect(fixedColumn.classes()).toContain('issues-column--drop-disabled');
    const inProgressColumn = w.find('[data-testid="issues-column-in_progress"]');
    expect(inProgressColumn.classes()).not.toContain('issues-column--drop-disabled');
  });

  it('dropping a card on a valid column moves it', async () => {
    api.updateIssueStatus.mockResolvedValue({});
    const { w } = await mountView();
    const card = w.find('[data-testid="issue-card"]');
    await card.trigger('dragstart');
    await w.find('[data-testid="issues-column-in_progress"]').trigger('drop');
    await flushPromises();

    expect(api.updateIssueStatus).toHaveBeenCalledWith('proj-1', 'i1', 'in_progress');
  });

  it('dropping a card on an invalid column does not move it', async () => {
    const { w } = await mountView();
    const card = w.find('[data-testid="issue-card"]');
    await card.trigger('dragstart');
    await w.find('[data-testid="issues-column-fixed"]').trigger('drop');
    await flushPromises();

    expect(api.updateIssueStatus).not.toHaveBeenCalled();
  });

  it('navigates to the issue detail page when a card is clicked', async () => {
    const { w, router } = await mountView();
    await w.find('[data-testid="issue-card"]').trigger('click');
    await flushPromises();
    expect(router.currentRoute.value.path).toBe('/issues/i1');
  });
});

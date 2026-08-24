import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createRouter, createMemoryHistory } from 'vue-router';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getChangeRequest:           vi.fn(),
    discardChangeRequest:       vi.fn(),
    applyChangeRequest:         vi.fn(),
    createChangeRequestComment: vi.fn(),
    redeployFromIssue:          vi.fn(),
    getProjectDeploymentSingle: vi.fn(),
    getArtifact:                vi.fn(),
    listProjectAgents:          vi.fn(),
  };
});

vi.mock('../../src/components/deployment/IssueChat.vue', () => ({
  default: { template: '<div class="stub-issue-chat" />', props: ['projectId', 'issueId', 'history', 'proposedFiles', 'proposedSummary', 'beforeFiles'] },
}));

import IssueDetailView from '../../src/views/IssueDetailView.vue';
import IssueChat from '../../src/components/deployment/IssueChat.vue';
import * as api from '../../src/lib/api.js';

const BASE_ISSUE = {
  id: 'cr1', title: 'Apply fails on security group', description: 'Details about **the** failure',
  status: 'open', kind: 'issue', proposed_solution_summary: null, proposed_files: null,
  comments: [], chat_messages: [], runs: [],
  deployment: { id: 'd1', name: 'Acme rollout', infra_state: 'broken' },
};

const DEPLOYMENT = {
  id: 'd1', name: 'Acme rollout', infra_state: 'broken',
  terraform_bundle: { id: 'a1', title: 'Infra', kind: 'terraform' },
};

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/change-requests/:id', component: IssueDetailView },
      { path: '/deploy', component: { template: '<div />' } },
    ],
  });
}

async function mountView({ issue = BASE_ISSUE, deployment = DEPLOYMENT, agents = [{ id: 'agent-1', hostname: 'host-1' }] } = {}) {
  api.getChangeRequest.mockResolvedValue(structuredClone(issue));
  api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(deployment));
  api.getArtifact.mockResolvedValue({ content: JSON.stringify({ 'main.tf': 'resource "x" {}' }) });
  api.listProjectAgents.mockResolvedValue(agents);

  const router = makeRouter();
  router.push('/change-requests/cr1');
  await router.isReady();
  const w = mount(IssueDetailView, {
    props: { projectId: 'proj-1' },
    global: { plugins: [router] },
  });
  await flushPromises();
  return { w, router };
}

describe('IssueDetailView (Change Request detail)', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('renders the change request title, status, and description', async () => {
    const { w } = await mountView();
    expect(w.text()).toContain('Apply fails on security group');
    expect(w.find('[data-testid="issue-status-badge"]').text()).toBe('Open');
    expect(w.find('[data-testid="issue-description"]').html()).toContain('<strong>the</strong>');
  });

  it('links to the deployment page', async () => {
    const { w } = await mountView();
    const link = w.find('[data-testid="view-deployment-link"]');
    expect(link.attributes('href')).toBe('/deploy');
    expect(link.text()).toContain('Acme rollout');
  });

  it('renders related runs via RunHistory', async () => {
    const { w } = await mountView({
      issue: { ...BASE_ISSUE, runs: [{ id: 'r1', action: 'apply', status: 'failed', exit_code: 1, stdout_preview: '', stderr_preview: 'boom', initiated_by: 'user', reasoning: null, created_at: '2026-08-07T12:00:00Z' }] },
    });
    expect(w.find('[data-testid="run-history"]').exists()).toBe(true);
    expect(w.text()).toContain('boom');
  });

  it('does not show the proposed-solution panel when no solution is proposed', async () => {
    const { w } = await mountView();
    expect(w.find('[data-testid="proposed-solution-panel"]').exists()).toBe(false);
  });

  it('shows the proposed-solution panel with a diff when a solution is proposed', async () => {
    const { w } = await mountView({
      issue: { ...BASE_ISSUE, proposed_solution_summary: 'widened the security group', proposed_files: { 'main.tf': 'resource "y" {}' } },
    });
    const panel = w.find('[data-testid="proposed-solution-panel"]');
    expect(panel.exists()).toBe(true);
    expect(panel.text()).toContain('widened the security group');
  });

  it('apply-solution button calls applyChangeRequest with the selected agent and reloads', async () => {
    api.applyChangeRequest.mockResolvedValue({ change_request: {}, redeploy: {} });
    const { w } = await mountView({
      issue: { ...BASE_ISSUE, proposed_solution_summary: 'fix', proposed_files: { 'main.tf': 'fixed' } },
    });

    await w.find('[data-testid="apply-solution-btn"]').trigger('click');
    await flushPromises();

    expect(api.applyChangeRequest).toHaveBeenCalledWith('proj-1', 'cr1', { agent_id: 'agent-1' });
    expect(api.getChangeRequest).toHaveBeenCalledTimes(2);
  });

  it('renders the comment list and posts a new comment', async () => {
    const updated = { ...BASE_ISSUE, comments: [{ id: 'c1', author_type: 'user', author_name: 'Alice', body: 'noted', created_at: '2026-08-07T12:00:00Z' }] };
    api.createChangeRequestComment.mockResolvedValue(updated);
    const { w } = await mountView();

    await w.find('[data-testid="issue-comment-input"]').setValue('noted');
    await w.find('[data-testid="post-comment-btn"]').trigger('click');
    await flushPromises();

    expect(api.createChangeRequestComment).toHaveBeenCalledWith('proj-1', 'cr1', 'noted');
    expect(w.text()).toContain('noted');
  });

  it('passes chat history and proposed solution down to IssueChat', async () => {
    const { w } = await mountView({
      issue: { ...BASE_ISSUE, chat_messages: [{ role: 'user', text: 'hi' }], proposed_files: { 'main.tf': 'fixed' }, proposed_solution_summary: 'fix' },
    });
    const chat = w.findComponent(IssueChat);
    expect(chat.exists()).toBe(true);
    expect(chat.props('issueId')).toBe('cr1');
    expect(chat.props('history')).toEqual([{ role: 'user', text: 'hi' }]);
    expect(chat.props('proposedFiles')).toEqual({ 'main.tf': 'fixed' });
    expect(chat.props('proposedSummary')).toBe('fix');
  });

  it('offers the open->in_review move button for an open change request', async () => {
    const { w } = await mountView();
    expect(w.find('[data-testid="move-issue-in_review"]').exists()).toBe(true);
    expect(w.find('[data-testid="move-issue-applied"]').exists()).toBe(false);
  });

  it('discarding a change request calls discardChangeRequest', async () => {
    api.discardChangeRequest.mockResolvedValue({});
    const { w } = await mountView({ issue: { ...BASE_ISSUE, status: 'in_review' } });
    await w.find('[data-testid="move-issue-discarded"]').trigger('click');
    await flushPromises();
    expect(api.discardChangeRequest).toHaveBeenCalledWith('proj-1', 'cr1');
  });

  it('redeploy button calls redeployFromIssue with the selected agent', async () => {
    api.redeployFromIssue.mockResolvedValue({ runs: [] });
    const { w } = await mountView();
    await w.find('[data-testid="issue-redeploy-btn"]').trigger('click');
    await flushPromises();
    expect(api.redeployFromIssue).toHaveBeenCalledWith('proj-1', 'cr1', { agent_id: 'agent-1' });
  });

  it('disables apply-solution and redeploy buttons when no agent is available', async () => {
    const { w } = await mountView({
      issue: { ...BASE_ISSUE, proposed_solution_summary: 'fix', proposed_files: { 'main.tf': 'fixed' } },
      agents: [],
    });
    expect(w.find('[data-testid="apply-solution-btn"]').attributes('disabled')).toBeDefined();
    expect(w.find('[data-testid="issue-redeploy-btn"]').attributes('disabled')).toBeDefined();
  });
});

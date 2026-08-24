import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    listDeploymentProposals: vi.fn(),
    approveProposal:         vi.fn(),
    discardProposal:         vi.fn(),
  };
});

vi.mock('../../src/components/deployment/DiffView.vue', () => ({
  default: { template: '<div class="stub-diff-view" />', props: ['before', 'after'] },
}));

import ReviewInbox from '../../src/components/deployment/ReviewInbox.vue';
import * as api from '../../src/lib/api.js';

const PROPOSALS = [
  {
    id: 'p1', source: 'prompt', explanation: 'Added security group',
    target_artifact_id: 'a1', target_artifact_kind: 'terraform',
    current_content: '{"main.tf":"old"}', proposed_content: '{"main.tf":"new"}',
    status: 'pending',
  },
  {
    id: 'p2', source: 'context', explanation: 'Updated design',
    target_artifact_id: 'a2', target_artifact_kind: 'markdown',
    current_content: '{"doc.md":"old"}', proposed_content: '{"doc.md":"new"}',
    status: 'approved',
  },
];

function mountInbox() {
  return mount(ReviewInbox, {
    props: { projectId: 'proj-1', deploymentId: 'd1' },
  });
}

describe('ReviewInbox', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('shows empty message when no proposals', async () => {
    api.listDeploymentProposals.mockResolvedValue([]);
    const w = mountInbox();
    await flushPromises();
    expect(w.text()).toContain('No proposals');
  });

  it('lists proposals with source, kind, and explanation', async () => {
    api.listDeploymentProposals.mockResolvedValue(PROPOSALS);
    const w = mountInbox();
    await flushPromises();
    expect(w.find('[data-testid="proposal-p1"]').exists()).toBe(true);
    expect(w.find('[data-testid="proposal-p2"]').exists()).toBe(true);
    expect(w.text()).toContain('Added security group');
    expect(w.text()).toContain('Updated design');
  });

  it('pending proposal shows approve/edit/discard buttons', async () => {
    api.listDeploymentProposals.mockResolvedValue(PROPOSALS);
    const w = mountInbox();
    await flushPromises();
    const item = w.find('[data-testid="proposal-p1"]');
    expect(item.text()).toContain('Approve');
    expect(item.text()).toContain('Discard');
    expect(item.text()).toContain('Edit');
  });

  it('non-pending proposal shows status instead of action buttons', async () => {
    api.listDeploymentProposals.mockResolvedValue(PROPOSALS);
    const w = mountInbox();
    await flushPromises();
    const item = w.find('[data-testid="proposal-p2"]');
    expect(item.text()).toContain('approved');
    expect(item.text()).not.toContain('Approve');
  });

  it('clicking show diff expands the DiffView', async () => {
    api.listDeploymentProposals.mockResolvedValue(PROPOSALS);
    const w = mountInbox();
    await flushPromises();
    const item = w.find('[data-testid="proposal-p1"]');
    expect(item.find('.stub-diff-view').exists()).toBe(false);
    await item.findAll('button').find(b => b.text().includes('Show diff')).trigger('click');
    expect(w.find('[data-testid="proposal-p1"]').find('.stub-diff-view').exists()).toBe(true);
  });

  it('approve calls approveProposal and reloads', async () => {
    api.listDeploymentProposals.mockResolvedValue(PROPOSALS);
    api.approveProposal.mockResolvedValue({ status: 'approved' });
    const w = mountInbox();
    await flushPromises();
    api.listDeploymentProposals.mockResolvedValue([]);
    await w.find('[data-testid="proposal-p1"]').findAll('button').find(b => b.text().includes('Approve')).trigger('click');
    await flushPromises();
    expect(api.approveProposal).toHaveBeenCalledWith('proj-1', 'd1', 'p1', {});
  });

  it('discard calls discardProposal and reloads', async () => {
    api.listDeploymentProposals.mockResolvedValue(PROPOSALS);
    api.discardProposal.mockResolvedValue(undefined);
    const w = mountInbox();
    await flushPromises();
    api.listDeploymentProposals.mockResolvedValue([]);
    await w.find('[data-testid="proposal-p1"]').findAll('button').find(b => b.text().includes('Discard')).trigger('click');
    await flushPromises();
    expect(api.discardProposal).toHaveBeenCalledWith('proj-1', 'd1', 'p1');
  });

  it('edit opens textarea with proposed content, apply calls approveProposal with edited content', async () => {
    api.listDeploymentProposals.mockResolvedValue(PROPOSALS);
    api.approveProposal.mockResolvedValue({ status: 'approved' });
    const w = mountInbox();
    await flushPromises();
    const item = w.find('[data-testid="proposal-p1"]');
    await item.findAll('button').find(b => b.text().includes('Edit')).trigger('click');
    expect(item.find('[data-testid="edit-content"]').exists()).toBe(true);
    const textarea = item.find('[data-testid="edit-content"]');
    await textarea.setValue('{"main.tf":"user-edited"}');
    api.listDeploymentProposals.mockResolvedValue([]);
    await item.findAll('button').find(b => b.text() === 'Apply').trigger('click');
    await flushPromises();
    expect(api.approveProposal).toHaveBeenCalledWith('proj-1', 'd1', 'p1', { edited_content: '{"main.tf":"user-edited"}' });
  });
});

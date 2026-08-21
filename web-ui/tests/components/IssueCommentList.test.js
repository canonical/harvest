import { describe, it, expect } from 'vitest';
import { mount } from '@vue/test-utils';

import IssueCommentList from '../../src/components/deployment/IssueCommentList.vue';

const COMMENTS = [
  { id: 'c1', author_type: 'harvest', author_name: 'Harvest', body: 'Created from a failed run.', created_at: '2026-08-07T12:00:00Z' },
  { id: 'c2', author_type: 'user', author_name: 'Alice', body: 'Looking into this.', created_at: '2026-08-07T13:00:00Z' },
];

describe('IssueCommentList', () => {
  it('shows an empty state with no comments', () => {
    const w = mount(IssueCommentList, { props: { comments: [] } });
    expect(w.text()).toContain('No activity yet.');
    expect(w.find('[data-testid="issue-comment"]').exists()).toBe(false);
  });

  it('renders every comment with its author and body', () => {
    const w = mount(IssueCommentList, { props: { comments: COMMENTS } });
    const items = w.findAll('[data-testid="issue-comment"]');
    expect(items).toHaveLength(2);
    expect(w.text()).toContain('Created from a failed run.');
    expect(w.text()).toContain('Alice');
    expect(w.text()).toContain('Looking into this.');
  });

  it('badges harvest-authored comments but not user comments', () => {
    const w = mount(IssueCommentList, { props: { comments: COMMENTS } });
    const items = w.findAll('[data-testid="issue-comment"]');
    expect(items[0].find('[data-testid="harvest-badge"]').exists()).toBe(true);
    expect(items[1].find('[data-testid="harvest-badge"]').exists()).toBe(false);
  });

  it('renders comment body as markdown', () => {
    const w = mount(IssueCommentList, {
      props: { comments: [{ id: 'c1', author_type: 'user', author_name: 'Alice', body: '**bold**', created_at: null }] },
    });
    expect(w.find('.issue-comments__body').html()).toContain('<strong>bold</strong>');
  });

  it('emits post-comment with the trimmed textarea value and clears it', async () => {
    const w = mount(IssueCommentList, { props: { comments: [] } });
    const textarea = w.find('[data-testid="issue-comment-input"]');
    await textarea.setValue('  fixed the typo  ');
    await w.find('[data-testid="post-comment-btn"]').trigger('click');

    expect(w.emitted('post-comment')).toEqual([['fixed the typo']]);
    expect(textarea.element.value).toBe('');
  });

  it('disables the post button when the draft is empty or whitespace-only', async () => {
    const w = mount(IssueCommentList, { props: { comments: [] } });
    expect(w.find('[data-testid="post-comment-btn"]').attributes('disabled')).toBeDefined();

    await w.find('[data-testid="issue-comment-input"]').setValue('   ');
    expect(w.find('[data-testid="post-comment-btn"]').attributes('disabled')).toBeDefined();

    await w.find('[data-testid="issue-comment-input"]').setValue('note');
    expect(w.find('[data-testid="post-comment-btn"]').attributes('disabled')).toBeUndefined();
  });
});

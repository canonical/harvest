import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    sendIssueChatMessage: vi.fn(),
    openProjectEvents:    vi.fn(),
  };
});

import IssueChat from '../../src/components/deployment/IssueChat.vue';
import * as api from '../../src/lib/api.js';

function mountChat(props = {}) {
  const close = vi.fn();
  api.openProjectEvents.mockReturnValue({ close });
  return mount(IssueChat, {
    props: {
      projectId: 'proj-1',
      issueId:   'issue-1',
      history:   [],
      ...props,
    },
  });
}

describe('IssueChat', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('renders prior chat history from the history prop', () => {
    api.openProjectEvents.mockReturnValue({ close: vi.fn() });
    const w = mountChat({
      history: [
        { role: 'user', text: 'what broke?' },
        { role: 'assistant', text: 'the security group', sources: [], tool_calls_made: 0 },
      ],
    });
    expect(w.text()).toContain('what broke?');
    expect(w.text()).toContain('the security group');
  });

  it('sends a message, appends the response, and refreshes', async () => {
    api.sendIssueChatMessage.mockResolvedValue({ answer: 'It was the security group.', chain: [], proposed_solution: false });
    const w = mountChat();

    await w.find('[data-testid="issue-chat-input"]').setValue('what broke?');
    await w.find('[data-testid="issue-chat-send-btn"]').trigger('click');
    await flushPromises();

    expect(api.sendIssueChatMessage).toHaveBeenCalledWith('proj-1', 'issue-1', 'what broke?');
    expect(w.text()).toContain('what broke?');
    expect(w.text()).toContain('It was the security group.');
    expect(w.emitted('refresh')).toBeTruthy();
  });

  it('clears the input after sending', async () => {
    api.sendIssueChatMessage.mockResolvedValue({ answer: 'ok', chain: [], proposed_solution: false });
    const w = mountChat();
    const input = w.find('[data-testid="issue-chat-input"]');
    await input.setValue('investigate please');
    await w.find('[data-testid="issue-chat-send-btn"]').trigger('click');
    await flushPromises();
    expect(input.element.value).toBe('');
  });

  it('disables the send button while a message is empty or in flight', async () => {
    let resolveSend;
    api.sendIssueChatMessage.mockReturnValue(new Promise((r) => { resolveSend = r; }));
    const w = mountChat();
    expect(w.find('[data-testid="issue-chat-send-btn"]').attributes('disabled')).toBeDefined();

    await w.find('[data-testid="issue-chat-input"]').setValue('go');
    expect(w.find('[data-testid="issue-chat-send-btn"]').attributes('disabled')).toBeUndefined();

    await w.find('[data-testid="issue-chat-send-btn"]').trigger('click');
    expect(w.find('[data-testid="issue-chat-send-btn"]').attributes('disabled')).toBeDefined();

    resolveSend({ answer: 'done', chain: [], proposed_solution: false });
    await flushPromises();
    expect(w.find('[data-testid="issue-chat-send-btn"]').attributes('disabled')).toBeDefined();
  });

  it('shows an error notification when the request fails', async () => {
    api.sendIssueChatMessage.mockRejectedValue(new Error('agent unreachable'));
    const w = mountChat();
    await w.find('[data-testid="issue-chat-input"]').setValue('go');
    await w.find('[data-testid="issue-chat-send-btn"]').trigger('click');
    await flushPromises();
    expect(w.text()).toContain('agent unreachable');
  });

  it('renders live thinking/tool-call trace events tagged with this issue id while sending', async () => {
    let capturedHandler;
    api.openProjectEvents.mockImplementation((projectId, convId, onEvent) => {
      capturedHandler = onEvent;
      return { close: vi.fn() };
    });
    let resolveSend;
    api.sendIssueChatMessage.mockReturnValue(new Promise((r) => { resolveSend = r; }));

    const w = mount(IssueChat, { props: { projectId: 'proj-1', issueId: 'issue-1', history: [] } });
    await w.find('[data-testid="issue-chat-input"]').setValue('go');
    await w.find('[data-testid="issue-chat-send-btn"]').trigger('click');

    capturedHandler({ issue_id: 'issue-1', type: 'tool_call', name: 'run_command', input: { command: 'cat log' } });
    await flushPromises();
    expect(w.text()).toContain('cat log');

    resolveSend({ answer: 'done', chain: [], proposed_solution: false });
    await flushPromises();
  });

  it('ignores trace events tagged with a different issue id', async () => {
    let capturedHandler;
    api.openProjectEvents.mockImplementation((projectId, convId, onEvent) => {
      capturedHandler = onEvent;
      return { close: vi.fn() };
    });
    let resolveSend;
    api.sendIssueChatMessage.mockReturnValue(new Promise((r) => { resolveSend = r; }));

    const w = mount(IssueChat, { props: { projectId: 'proj-1', issueId: 'issue-1', history: [] } });
    await w.find('[data-testid="issue-chat-input"]').setValue('go');
    await w.find('[data-testid="issue-chat-send-btn"]').trigger('click');

    capturedHandler({ issue_id: 'other-issue', type: 'tool_call', name: 'run_command', input: { command: 'cat log' } });
    await flushPromises();
    expect(w.find('.tc-step').exists()).toBe(false);

    resolveSend({ answer: 'done', chain: [], proposed_solution: false });
    await flushPromises();
  });

  it('shows the proposed diff with an Approve button when a turn proposes a solution', async () => {
    api.sendIssueChatMessage.mockResolvedValue({ answer: 'Here is a fix.', chain: [], proposed_solution: true });
    const w = mountChat({
      proposedFiles: { 'main.tf': 'fixed' },
      proposedSummary: 'widened the security group rule',
      beforeFiles: { 'main.tf': 'broken' },
    });

    expect(w.find('[data-testid="issue-chat-proposed-diff"]').exists()).toBe(false);

    await w.find('[data-testid="issue-chat-input"]').setValue('please fix it');
    await w.find('[data-testid="issue-chat-send-btn"]').trigger('click');
    await flushPromises();

    expect(w.find('[data-testid="issue-chat-proposed-diff"]').exists()).toBe(true);
    expect(w.text()).toContain('widened the security group rule');
  });

  it('emits approve when the Approve button is clicked', async () => {
    api.sendIssueChatMessage.mockResolvedValue({ answer: 'Here is a fix.', chain: [], proposed_solution: true });
    const w = mountChat({
      proposedFiles: { 'main.tf': 'fixed' },
      proposedSummary: 'fix',
      beforeFiles: { 'main.tf': 'broken' },
    });
    await w.find('[data-testid="issue-chat-input"]').setValue('please fix it');
    await w.find('[data-testid="issue-chat-send-btn"]').trigger('click');
    await flushPromises();

    await w.find('[data-testid="issue-chat-approve-btn"]').trigger('click');
    expect(w.emitted('approve')).toBeTruthy();
  });

  it('does not show a stale proposed diff after a follow-up turn that proposes nothing new', async () => {
    api.sendIssueChatMessage
      .mockResolvedValueOnce({ answer: 'Here is a fix.', chain: [], proposed_solution: true })
      .mockResolvedValueOnce({ answer: 'Just checking something.', chain: [], proposed_solution: false });
    const w = mountChat({
      proposedFiles: { 'main.tf': 'fixed' },
      proposedSummary: 'fix',
      beforeFiles: { 'main.tf': 'broken' },
    });

    await w.find('[data-testid="issue-chat-input"]').setValue('please fix it');
    await w.find('[data-testid="issue-chat-send-btn"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="issue-chat-proposed-diff"]').exists()).toBe(true);

    await w.find('[data-testid="issue-chat-input"]').setValue('one more question');
    await w.find('[data-testid="issue-chat-send-btn"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="issue-chat-proposed-diff"]').exists()).toBe(false);
  });
});

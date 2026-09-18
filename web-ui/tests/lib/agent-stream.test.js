import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { useAgentStream } from '../../src/lib/agent-stream.js';

describe('useAgentStream — status text', () => {
  it('starts with the preparing text', () => {
    const s = useAgentStream({ preparingText: 'Preparing…' });
    expect(s.statusText.value).toBe('Preparing…');
  });

  it('shows the running tool description while a tool call is in flight', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'tool_call', name: 'list_repositories', input: {} });
    expect(s.statusText.value).toContain('Discovering available repositories');
  });

  it('shows Thinking… while a thinking block is streaming with nothing else running', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'thinking_delta', text: 'hmm' });
    expect(s.statusText.value).toBe('Thinking…');
  });

  it('shows the writing text once prose starts streaming', () => {
    const s = useAgentStream({ writingText: 'Writing…' });
    s.handleEvent({ type: 'text_delta', text: 'hello' });
    expect(s.statusText.value).toBe('Writing…');
  });

  it('shows the ready text once done', () => {
    const s = useAgentStream({ readyText: 'Ready!' });
    s.handleEvent({ type: 'done', answer: 'x' });
    expect(s.statusText.value).toBe('Ready!');
    expect(s.finished.value).toBe(true);
  });

  it('shows the failed text and records the message on error', () => {
    const s = useAgentStream({ failedText: 'Failed!' });
    s.handleEvent({ type: 'error', message: 'boom' });
    expect(s.statusText.value).toBe('Failed!');
    expect(s.error.value).toBe('boom');
    expect(s.finished.value).toBe(true);
  });

  it('shows a parallel-research status while leads are running', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'parallel_research_started', leads: ['a', 'b'] });
    expect(s.statusText.value).toBe('Researching 2 leads in parallel…');
  });

  it('shows a merging status once the fan-out starts merging, before any merge tool call fires', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'parallel_research_started', leads: ['a', 'b'] });
    s.handleEvent({ type: 'parallel_research_merge_started', duration_ms: 1000 });
    expect(s.statusText.value).toBe('Merging research findings…');
  });

  it('prefers a running tool call over a merging research status', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'parallel_research_started', leads: ['a', 'b'] });
    s.handleEvent({ type: 'parallel_research_merge_started', duration_ms: 1000 });
    s.handleEvent({ type: 'tool_call', name: 'list_repositories', input: {} });
    expect(s.statusText.value).toContain('Discovering available repositories');
  });
});

describe('useAgentStream — chain building', () => {
  it('accumulates thinking_delta text onto one streaming block', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'thinking_delta', text: 'a' });
    s.handleEvent({ type: 'thinking_delta', text: 'b' });
    expect(s.chain.value).toHaveLength(1);
    expect(s.chain.value[0]).toMatchObject({ type: 'thinking', text: 'ab', streaming: true });
  });

  it('finalizes the streaming thinking block once a tool call arrives', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'thinking_delta', text: 'a' });
    s.handleEvent({ type: 'tool_call', name: 'list_repositories', input: {} });
    expect(s.chain.value[0].streaming).toBe(false);
  });

  it('marks a running tool call done on its matching tool_result', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'tool_call', name: 'list_repositories', input: {} });
    s.handleEvent({ type: 'tool_result', name: 'list_repositories', preview: 'ok' });
    expect(s.chain.value[0]).toMatchObject({ status: 'done', preview: 'ok' });
  });

  it('counts tool calls made so far', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'tool_call', name: 'a', input: {} });
    s.handleEvent({ type: 'tool_call', name: 'b', input: {} });
    expect(s.toolCallCount.value).toBe(2);
  });

  it('tracks parallel research leads and their completion', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'parallel_research_started', leads: ['Networking', 'Sizing'] });
    s.handleEvent({ type: 'parallel_research_lead_done', index: 1, iterations: 2, duration_ms: 500, preview: 'done' });
    const block = s.chain.value.find(c => c.type === 'parallel_research');
    expect(block.leads[1]).toMatchObject({ status: 'done', iterations: 2, durationMs: 500, preview: 'done' });
    expect(block.leads[0].status).toBe('running');
  });
});

describe('useAgentStream — thinkingText', () => {
  it('is empty before any thinking arrives', () => {
    const s = useAgentStream();
    expect(s.thinkingText.value).toBe('');
  });

  it('accumulates thinking_delta chunks', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'thinking_delta', text: 'Looking at the ' });
    s.handleEvent({ type: 'thinking_delta', text: 'requested topology.' });
    expect(s.thinkingText.value).toBe('Looking at the requested topology.');
  });

  it('joins separate thinking segments interrupted by a tool call with a blank line', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'thinking_delta', text: 'First thought.' });
    s.handleEvent({ type: 'tool_call', name: 'list_repositories', input: {} });
    s.handleEvent({ type: 'thinking_delta', text: 'Second thought.' });
    expect(s.thinkingText.value).toBe('First thought.\n\nSecond thought.');
  });

  it('does not include tool call or parallel research chain entries', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'thinking_delta', text: 'Thinking.' });
    s.handleEvent({ type: 'tool_call', name: 'list_repositories', input: {} });
    s.handleEvent({ type: 'parallel_research_started', leads: ['a'] });
    expect(s.thinkingText.value).toBe('Thinking.');
  });

  it('clears on reset()', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'thinking_delta', text: 'Thinking.' });
    s.reset();
    expect(s.thinkingText.value).toBe('');
  });
});

describe('useAgentStream — timer', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('reset() starts the elapsed timer at 0 and ticks every second', () => {
    const s = useAgentStream();
    s.reset();
    expect(s.elapsedLabel.value).toBe('0s');
    vi.advanceTimersByTime(3000);
    expect(s.elapsedLabel.value).toBe('3s');
  });

  it('stops ticking once done', () => {
    const s = useAgentStream();
    s.reset();
    s.handleEvent({ type: 'done', answer: 'x' });
    vi.advanceTimersByTime(5000);
    expect(s.elapsedLabel.value).toBe('0s');
  });
});

describe('useAgentStream — file tracking', () => {
  function toolCall(input) {
    return { type: 'tool_call', name: 'generate_artifact', input };
  }

  it('creates a single-file entry for a bash artifact', () => {
    const s = useAgentStream();
    s.handleEvent(toolCall({ title: 'deploy-airbank.sh', kind: 'bash', content: '#!/bin/bash\necho hi' }));
    expect(s.files.value).toHaveLength(1);
    expect(s.files.value[0]).toMatchObject({
      title: 'deploy-airbank.sh',
      kind: 'bash',
      status: 'saving',
      subfiles: [{ path: 'deploy-airbank.sh', text: '#!/bin/bash\necho hi' }],
    });
  });

  it('sets the new file as the active file', () => {
    const s = useAgentStream();
    s.handleEvent(toolCall({ title: 'a.sh', kind: 'bash', content: 'echo a' }));
    const firstId = s.files.value[0].id;
    expect(s.activeFileId.value).toBe(firstId);
    s.handleEvent(toolCall({ title: 'b.sh', kind: 'bash', content: 'echo b' }));
    expect(s.activeFileId.value).toBe(s.files.value[1].id);
    expect(s.activeFileId.value).not.toBe(firstId);
  });

  it('expands a terraform bundle into one subfile per path', () => {
    const s = useAgentStream();
    s.handleEvent(toolCall({
      title: 'Infra',
      kind: 'terraform',
      content: JSON.stringify({ 'main.tf': 'resource "x" {}', 'variables.tf': 'variable "x" {}' }),
    }));
    expect(s.files.value[0].subfiles).toEqual([
      { path: 'main.tf', text: 'resource "x" {}' },
      { path: 'variables.tf', text: 'variable "x" {}' },
    ]);
  });

  it('falls back to one opaque blob when terraform content is not valid JSON', () => {
    const s = useAgentStream();
    s.handleEvent(toolCall({ title: 'Infra', kind: 'terraform', content: 'not json' }));
    expect(s.files.value[0].subfiles).toEqual([{ path: 'Infra', text: 'not json' }]);
  });

  it('marks a file saved on its matching tool_result', () => {
    const s = useAgentStream();
    s.handleEvent(toolCall({ title: 'a.sh', kind: 'bash', content: 'echo a' }));
    s.handleEvent({ type: 'tool_result', name: 'generate_artifact', preview: '{}' });
    expect(s.files.value[0].status).toBe('saved');
  });

  it('marks the earliest still-saving file done when results arrive in call order', () => {
    const s = useAgentStream();
    s.handleEvent(toolCall({ title: 'a.sh', kind: 'bash', content: 'echo a' }));
    s.handleEvent(toolCall({ title: 'b.sh', kind: 'bash', content: 'echo b' }));
    s.handleEvent({ type: 'tool_result', name: 'generate_artifact', preview: '{}' });
    expect(s.files.value[0].status).toBe('saved');
    expect(s.files.value[1].status).toBe('saving');
  });

  it('updates an existing file in place when the same artifact_id is revised', () => {
    const s = useAgentStream();
    s.handleEvent(toolCall({ title: 'a.sh', kind: 'bash', content: 'echo a', artifact_id: 'art-1' }));
    const id = s.files.value[0].id;
    s.handleEvent(toolCall({ title: 'a.sh', kind: 'bash', content: 'echo a v2', artifact_id: 'art-1' }));
    expect(s.files.value).toHaveLength(1);
    expect(s.files.value[0].id).toBe(id);
    expect(s.files.value[0].subfiles[0].text).toBe('echo a v2');
  });

  it('ignores non-generate_artifact tool calls', () => {
    const s = useAgentStream();
    s.handleEvent({ type: 'tool_call', name: 'set_execution_plan', input: {} });
    expect(s.files.value).toEqual([]);
  });

  it('resets files and activeFileId on reset()', () => {
    const s = useAgentStream();
    s.handleEvent(toolCall({ title: 'a.sh', kind: 'bash', content: 'echo a' }));
    s.reset();
    expect(s.files.value).toEqual([]);
    expect(s.activeFileId.value).toBe(null);
  });
});

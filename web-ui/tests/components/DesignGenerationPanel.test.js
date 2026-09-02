import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    generateDesignStream: vi.fn(),
  };
});

import DesignGenerationPanel from '../../src/components/deployment/DesignGenerationPanel.vue';
import * as api from '../../src/lib/api.js';

function mountPanel({ projectId = 'proj-1', deploymentId = 'd1', body = {} } = {}) {
  return mount(DesignGenerationPanel, {
    props: { projectId, deploymentId, body },
  });
}

// The real endpoint stays open until the stream naturally ends; done/error are
// conveyed as events, not by resolving the fetch promise. Mirror that here so
// tests control completion purely through emitted events.
function makeStreamController() {
  let onEvent;
  api.generateDesignStream.mockImplementation((_p, _d, _b, cb) => {
    onEvent = cb;
    return new Promise(() => {});
  });
  return { get onEvent() { return onEvent; } };
}

describe('DesignGenerationPanel', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    api.generateDesignStream.mockImplementation(() => new Promise(() => {}));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders a single status line, starting in a "Preparing…" state', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="design-generation"]').exists()).toBe(true);
    expect(w.find('[data-testid="design-gen-status-text"]').text()).toMatch(/preparing/i);
    expect(w.find('.loading-orbit').exists()).toBe(true);
  });

  it('calls generateDesignStream on mount with projectId, deploymentId, and body', async () => {
    mountPanel({ projectId: 'proj-1', deploymentId: 'd1', body: { artifact_ids: ['a1'], product_template_id: 't1' } });
    await flushPromises();
    expect(api.generateDesignStream).toHaveBeenCalledWith('proj-1', 'd1', {
      artifact_ids: ['a1'], product_template_id: 't1',
    }, expect.any(Function));
  });

  it('shows an increasing elapsed time while generation is running', async () => {
    vi.useFakeTimers();
    const w = mountPanel();
    await vi.advanceTimersByTimeAsync(0);
    expect(w.find('[data-testid="design-gen-elapsed"]').text()).toBe('0s');
    await vi.advanceTimersByTimeAsync(3000);
    expect(w.find('[data-testid="design-gen-elapsed"]').text()).toBe('3s');
  });

  it('updates the status line to what the currently running tool is doing, in its own words', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'tool_call', name: 'list_repositories', input: {} });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-status-text"]').text()).toContain('Discovering available repositories');
  });

  it('shows a "Thinking…" status while the model is reasoning with no tool running', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'thinking_delta', text: 'Considering the options' });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-status-text"]').text()).toMatch(/thinking/i);
  });

  it('switches the status line to "Writing…" once the document starts streaming', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: {} });
    ctl.onEvent({ type: 'tool_result', name: 'generate_artifact', preview: '{}' });
    ctl.onEvent({ type: 'text_delta', text: '# Design\n' });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-status-text"]').text()).toMatch(/writing/i);
  });

  it('streams text deltas into a live markdown preview', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'text_delta', text: '# Design\n' });
    ctl.onEvent({ type: 'text_delta', text: 'Single VM.' });
    await flushPromises();
    const preview = w.find('[data-testid="design-gen-preview"]');
    expect(preview.exists()).toBe(true);
    expect(preview.html()).toContain('<h1');
    expect(preview.text()).toContain('Single VM.');
  });

  it('shows no activity section until there is activity to show', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="design-gen-activity"]').exists()).toBe(false);
  });

  it('shows the activity log, permanently, once there is something to show', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: { title: 'Design' } });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-activity"]').exists()).toBe(true);
    expect(w.find('[data-testid="design-gen-timeline"]').text()).toContain('Generating artifact');
  });

  it('reveals thinking blocks inside the details panel as soon as they arrive', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'thinking_delta', text: 'Let me think...' });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-thinking"]').exists()).toBe(true);
  });

  it('auto-scrolls the activity region to the bottom when new tool calls arrive', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: {} });
    await flushPromises();

    const activity = w.find('[data-testid="design-gen-activity"]').element;
    Object.defineProperty(activity, 'scrollHeight', { configurable: true, get: () => 500 });
    Object.defineProperty(activity, 'clientHeight', { configurable: true, get: () => 200 });
    for (let i = 0; i < 5; i++) {
      ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: {} });
      await flushPromises();
    }
    expect(activity.scrollTop).toBe(500);
  });

  it('shows a ready state and emits done when the done event arrives', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'done', answer: '# Design' });
    await flushPromises();
    expect(w.emitted('done')).toBeTruthy();
    expect(w.find('[data-testid="design-gen-status-text"]').text()).toMatch(/ready/i);
    expect(w.find('.loading-orbit').exists()).toBe(false);
    expect(w.find('[data-testid="design-gen-elapsed"]').exists()).toBe(false);
  });

  it('shows an error with recovery actions and does not auto-navigate away', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'error', message: 'LLM failed' });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-error"]').text()).toContain('LLM failed');
    expect(w.find('[data-testid="design-gen-status-text"]').text()).toMatch(/failed/i);
    expect(w.find('[data-testid="design-gen-back"]').exists()).toBe(true);
    expect(w.find('[data-testid="design-gen-retry"]').exists()).toBe(true);
    expect(w.emitted('done')).toBeFalsy();
  });

  it('shows an error when the stream promise rejects, without auto-navigating away', async () => {
    api.generateDesignStream.mockRejectedValue(new Error('Network down'));
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="design-gen-error"]').text()).toContain('Network down');
    expect(w.emitted('done')).toBeFalsy();
  });

  it('Back emits cancel so the caller can return to setup', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'error', message: 'LLM failed' });
    await flushPromises();
    await w.find('[data-testid="design-gen-back"]').trigger('click');
    expect(w.emitted('cancel')).toBeTruthy();
  });

  it('uses a custom streamFn instead of generateDesignStream when supplied', async () => {
    const customStream = vi.fn(() => new Promise(() => {}));
    mount(DesignGenerationPanel, {
      props: { projectId: 'proj-1', deploymentId: 'd1', body: { explanation: 'Add a CDN' }, streamFn: customStream },
    });
    await flushPromises();
    expect(customStream).toHaveBeenCalledWith('proj-1', 'd1', { explanation: 'Add a CDN' }, expect.any(Function));
    expect(api.generateDesignStream).not.toHaveBeenCalled();
  });

  it('emits done with the final answer and accumulated text from the stream', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'text_delta', text: '# Design\n' });
    ctl.onEvent({ type: 'done', answer: '# Design (final)' });
    await flushPromises();
    expect(w.emitted('done')[0][0]).toEqual({ answer: '# Design (final)', text: '# Design\n' });
  });

  it('uses custom preparing/ready/failed labels when provided', async () => {
    const ctl = makeStreamController();
    const w = mount(DesignGenerationPanel, {
      props: {
        projectId: 'proj-1', deploymentId: 'd1', body: {},
        preparingText: 'Preparing your proposed change…',
        readyText: 'Proposed change ready',
        failedText: 'Failed to propose change',
      },
    });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-status-text"]').text()).toBe('Preparing your proposed change…');
    ctl.onEvent({ type: 'done', answer: 'x' });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-status-text"]').text()).toBe('Proposed change ready');
  });

  it('Try again retries generation and resets the running state', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'error', message: 'LLM failed' });
    await flushPromises();

    await w.find('[data-testid="design-gen-retry"]').trigger('click');
    await flushPromises();

    expect(api.generateDesignStream).toHaveBeenCalledTimes(2);
    expect(w.find('[data-testid="design-gen-error"]').exists()).toBe(false);
    expect(w.find('.loading-orbit').exists()).toBe(true);

    ctl.onEvent({ type: 'done', answer: '# Design' });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-status-text"]').text()).toMatch(/ready/i);
  });
});

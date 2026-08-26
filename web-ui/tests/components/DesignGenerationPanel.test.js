import { describe, it, expect, vi, beforeEach } from 'vitest';
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

function makeStreamController() {
  let onEvent;
  api.generateDesignStream.mockImplementation(async (_p, _d, _b, cb) => {
    onEvent = cb;
  });
  return { get onEvent() { return onEvent; } };
}

describe('DesignGenerationPanel', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    api.generateDesignStream.mockImplementation(async () => {});
  });

  it('renders a generation status page with title', async () => {
    const { w } = { w: mountPanel() };
    await flushPromises();
    expect(w.find('[data-testid="design-generation"]').exists()).toBe(true);
    expect(w.text()).toMatch(/generating.*design/i);
  });

  it('calls generateDesignStream on mount with projectId, deploymentId, and body', async () => {
    mountPanel({ projectId: 'proj-1', deploymentId: 'd1', body: { artifact_ids: ['a1'], product_template_id: 't1' } });
    await flushPromises();
    expect(api.generateDesignStream).toHaveBeenCalledWith('proj-1', 'd1', {
      artifact_ids: ['a1'], product_template_id: 't1',
    }, expect.any(Function));
  });

  it('shows intent badge when intent event arrives', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'intent', mode: 'action' });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-intent"]').exists()).toBe(true);
  });

  it('shows phase label when phase event arrives', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'phase', label: 'Drafting design' });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-phase"]').text()).toContain('Drafting design');
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

  it('renders tool call steps as a vertical timeline', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: { title: 'Design' } });
    ctl.onEvent({ type: 'tool_result', name: 'generate_artifact', preview: '{"id":"a1"}' });
    await flushPromises();
    const timeline = w.find('[data-testid="design-gen-timeline"]');
    expect(timeline.exists()).toBe(true);
    expect(timeline.text()).toContain('Generating artifact');
  });

  it('renders thinking blocks when thinking events arrive', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'thinking_delta', text: 'Let me think...' });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-thinking"]').exists()).toBe(true);
  });

  it('groups thinking and tool timeline in a fixed activity region above the preview', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    const activity = w.find('[data-testid="design-gen-activity"]');
    expect(activity.exists()).toBe(true);
    ctl.onEvent({ type: 'thinking_delta', text: 'Considering options' });
    await flushPromises();
    expect(activity.find('[data-testid="design-gen-thinking"]').exists()).toBe(true);
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: {} });
    await flushPromises();
    expect(activity.find('[data-testid="design-gen-timeline"]').exists()).toBe(true);
    ctl.onEvent({ type: 'text_delta', text: '# Design' });
    await flushPromises();
    const root = w.find('.design-gen').element;
    const activityEl = activity.element;
    const previewWrapper = w.find('[data-testid="design-gen-preview"]').element.parentElement;
    const rootChildren = Array.from(root.children);
    expect(rootChildren.indexOf(activityEl)).toBeLessThan(rootChildren.indexOf(previewWrapper));
  });

  it('auto-scrolls the activity region to the bottom when new tool calls arrive', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
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

  it('emits done when done event arrives and shows completion state', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'done', answer: '# Design' });
    await flushPromises();
    expect(w.emitted('done')).toBeTruthy();
  });

  it('shows error and emits done when error event arrives', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'error', message: 'LLM failed' });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-error"]').exists()).toBe(true);
    expect(w.find('[data-testid="design-gen-error"]').text()).toContain('LLM failed');
    expect(w.emitted('done')).toBeTruthy();
  });

  it('shows error when the stream promise rejects', async () => {
    api.generateDesignStream.mockRejectedValue(new Error('Network down'));
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="design-gen-error"]').exists()).toBe(true);
    expect(w.find('[data-testid="design-gen-error"]').text()).toContain('Network down');
    expect(w.emitted('done')).toBeTruthy();
  });

  it('shows a spinner while waiting for the first event', async () => {
    api.generateDesignStream.mockImplementation(() => new Promise(() => {}));
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="design-gen-spinner"]').exists()).toBe(true);
  });
});

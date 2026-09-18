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

  it('does not show intent, phase, or tool-count badges — the status line alone carries the meaning now', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'intent', mode: 'research' });
    ctl.onEvent({ type: 'phase', label: 'Gathering context' });
    ctl.onEvent({ type: 'tool_call', name: 'list_repositories', input: {} });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-intent"]').exists()).toBe(false);
    expect(w.find('[data-testid="design-gen-phase"]').exists()).toBe(false);
    expect(w.find('[data-testid="design-gen-tool-count"]').exists()).toBe(false);
  });

  it('streams text into the document hero as it arrives', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'text_delta', text: '# Design\n' });
    ctl.onEvent({ type: 'text_delta', text: 'Single VM.' });
    await flushPromises();
    const hero = w.find('[data-testid="document-hero"]');
    expect(hero.exists()).toBe(true);
    expect(hero.html()).toContain('<h1');
    expect(hero.text()).toContain('Single VM.');
  });

  it('shows the model\'s thinking in the hero, styled distinctly, before any real content streams', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'thinking_delta', text: 'Looking at the requested topology.' });
    await flushPromises();
    const hero = w.find('[data-testid="document-hero"]');
    expect(hero.text()).toContain('Looking at the requested topology.');
    expect(hero.classes()).toContain('doc-hero--thinking');
  });

  it('keeps showing thinking across a tool call, joined into one transcript', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'thinking_delta', text: 'First thought.' });
    ctl.onEvent({ type: 'tool_call', name: 'list_repositories', input: {} });
    ctl.onEvent({ type: 'thinking_delta', text: 'Second thought.' });
    await flushPromises();
    const hero = w.find('[data-testid="document-hero"]');
    expect(hero.text()).toContain('First thought.');
    expect(hero.text()).toContain('Second thought.');
  });

  it('switches the hero from thinking to the real document once it starts streaming', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'thinking_delta', text: 'Looking at the requested topology.' });
    ctl.onEvent({ type: 'text_delta', text: '# Design\n' });
    await flushPromises();
    const hero = w.find('[data-testid="document-hero"]');
    expect(hero.text()).not.toContain('Looking at the requested topology.');
    expect(hero.classes()).not.toContain('doc-hero--thinking');
    expect(hero.html()).toContain('<h1');
  });

  it('shows the document hero with just a caret before any text has streamed', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="document-hero"]').exists()).toBe(true);
    expect(w.find('[data-testid="document-hero-block"]').exists()).toBe(false);
  });

  it('never shows a "how was this made" disclosure — the status line is the only window into activity now', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: { title: 'Design' } });
    ctl.onEvent({ type: 'thinking_delta', text: 'Let me think...' });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-details"]').exists()).toBe(false);
    expect(w.find('[data-testid="design-gen-activity"]').exists()).toBe(false);
  });

  it('walks the status line through each tool call as they run, so the latest one is always visible', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();

    ctl.onEvent({ type: 'tool_call', name: 'list_repositories', input: {} });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-status-text"]').text()).toContain('Discovering available repositories');

    ctl.onEvent({ type: 'tool_result', name: 'list_repositories', preview: '[]' });
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: { title: 'Design' } });
    await flushPromises();
    expect(w.find('[data-testid="design-gen-status-text"]').text()).toContain('Generating artifact Design');
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

  it('keeps any partial document visible underneath the error banner', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'text_delta', text: '# Design\n' });
    ctl.onEvent({ type: 'error', message: 'LLM failed' });
    await flushPromises();
    expect(w.find('[data-testid="document-hero"]').text()).toContain('Design');
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

  it('shows the header when eyebrow/title/badge are provided', async () => {
    const w = mount(DesignGenerationPanel, {
      props: {
        projectId: 'proj-1', deploymentId: 'd1', body: {},
        eyebrow: 'Design', title: 'AirBank53', subtitle: 'Generating your design document…', badge: 'Gateway',
      },
    });
    await flushPromises();
    expect(w.find('[data-testid="design-eyebrow"]').text()).toBe('Design');
    expect(w.text()).toContain('AirBank53');
    expect(w.text()).toContain('Gateway');
    expect(w.text()).toContain('Generating your design document…');
  });

  it('shows no header when no title is provided, as used inline for propose-a-change', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="design-eyebrow"]').exists()).toBe(false);
  });
});

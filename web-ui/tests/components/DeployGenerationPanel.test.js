import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    generateProvisionStream: vi.fn(),
  };
});

import DeployGenerationPanel from '../../src/components/deployment/DeployGenerationPanel.vue';
import * as api from '../../src/lib/api.js';

function mountPanel({ projectId = 'proj-1', deploymentId = 'd1', deploymentName = 'MyProject' } = {}) {
  return mount(DeployGenerationPanel, {
    props: { projectId, deploymentId, deploymentName },
  });
}

function makeStreamController() {
  let onEvent;
  api.generateProvisionStream.mockImplementation(async (_p, _d, cb) => {
    onEvent = cb;
  });
  return { get onEvent() { return onEvent; } };
}

describe('DeployGenerationPanel', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    api.generateProvisionStream.mockImplementation(async () => {});
  });

  it('renders a generation status page with title', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="deploy-generation"]').exists()).toBe(true);
    expect(w.text()).toMatch(/generating.*deployment artifacts/i);
  });

  it('shows the deployment name as subtitle', async () => {
    const w = mountPanel({ deploymentName: 'Rollout-42' });
    await flushPromises();
    expect(w.text()).toContain('Rollout-42');
  });

  it('calls generateProvisionStream on mount with projectId and deploymentId', async () => {
    mountPanel({ projectId: 'proj-1', deploymentId: 'd1' });
    await flushPromises();
    expect(api.generateProvisionStream).toHaveBeenCalledWith('proj-1', 'd1', expect.any(Function));
  });

  it('shows phase label when phase event arrives', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'phase', label: 'Writing Terraform' });
    await flushPromises();
    expect(w.find('[data-testid="deploy-gen-phase"]').text()).toContain('Writing Terraform');
  });

  it('shows intent badge when intent event arrives', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'intent', mode: 'action' });
    await flushPromises();
    expect(w.find('[data-testid="deploy-gen-intent"]').exists()).toBe(true);
  });

  it('renders tool call steps as a vertical timeline', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: { title: 'Infra' } });
    ctl.onEvent({ type: 'tool_result', name: 'generate_artifact', preview: '{"id":"a1"}' });
    await flushPromises();
    const timeline = w.find('[data-testid="deploy-gen-timeline"]');
    expect(timeline.exists()).toBe(true);
    expect(timeline.text()).toContain('Generating artifact');
  });

  it('renders thinking blocks when thinking events arrive', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'thinking_delta', text: 'Planning the bundle...' });
    await flushPromises();
    expect(w.find('[data-testid="deploy-gen-thinking"]').exists()).toBe(true);
  });

  it('emits done when done event arrives', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'done' });
    await flushPromises();
    expect(w.emitted('done')).toBeTruthy();
  });

  it('shows error and emits done when error event arrives', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'error', message: 'LLM failed' });
    await flushPromises();
    expect(w.find('[data-testid="deploy-gen-error"]').exists()).toBe(true);
    expect(w.find('[data-testid="deploy-gen-error"]').text()).toContain('LLM failed');
    expect(w.emitted('done')).toBeTruthy();
  });

  it('shows error when the stream promise rejects', async () => {
    api.generateProvisionStream.mockRejectedValue(new Error('Network down'));
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="deploy-gen-error"]').exists()).toBe(true);
    expect(w.find('[data-testid="deploy-gen-error"]').text()).toContain('Network down');
    expect(w.emitted('done')).toBeTruthy();
  });

  it('shows a spinner while waiting for the first event', async () => {
    api.generateProvisionStream.mockImplementation(() => new Promise(() => {}));
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="deploy-gen-spinner"]').exists()).toBe(true);
  });
});

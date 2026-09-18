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
  api.generateProvisionStream.mockImplementation((_p, _d, cb) => {
    onEvent = cb;
    return new Promise(() => {});
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

  it('shows the deployment name as the page title', async () => {
    const w = mountPanel({ deploymentName: 'Rollout-42' });
    await flushPromises();
    expect(w.text()).toContain('Rollout-42');
  });

  it('calls generateProvisionStream on mount with projectId and deploymentId', async () => {
    mountPanel({ projectId: 'proj-1', deploymentId: 'd1' });
    await flushPromises();
    expect(api.generateProvisionStream).toHaveBeenCalledWith('proj-1', 'd1', expect.any(Function));
  });

  it('does not show intent or phase badges — the status line alone carries the meaning now', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'intent', mode: 'action' });
    ctl.onEvent({ type: 'phase', label: 'Writing Terraform' });
    await flushPromises();
    expect(w.find('[data-testid="deploy-gen-intent"]').exists()).toBe(false);
    expect(w.find('[data-testid="deploy-gen-phase"]').exists()).toBe(false);
  });

  it('shows the model\'s thinking in the hero before any artifact has been generated', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'thinking_delta', text: 'Translating the design into infrastructure.' });
    await flushPromises();
    const hero = w.find('[data-testid="document-hero"]');
    expect(hero.exists()).toBe(true);
    expect(hero.text()).toContain('Translating the design into infrastructure.');
    expect(hero.classes()).toContain('doc-hero--thinking');
    expect(w.find('[data-testid="file-tab"]').exists()).toBe(false);
  });

  it('switches from the thinking hero to file tabs once the first artifact starts generating', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'thinking_delta', text: 'Translating the design into infrastructure.' });
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: { title: 'deploy.sh', kind: 'bash', content: 'echo hi' } });
    await flushPromises();
    expect(w.find('[data-testid="document-hero"]').exists()).toBe(false);
    expect(w.find('[data-testid="file-tab"]').exists()).toBe(true);
  });

  it('renders a file tab for each generated artifact', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: { title: 'deploy.sh', kind: 'bash', content: 'echo hi' } });
    await flushPromises();
    const tabs = w.findAll('[data-testid="file-tab"]');
    expect(tabs).toHaveLength(1);
    expect(tabs[0].text()).toContain('deploy.sh');
    expect(w.find('[data-testid="file-tab-content"]').text()).toContain('echo hi');
  });

  it('expands a terraform bundle into one tab per file', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({
      type: 'tool_call', name: 'generate_artifact',
      input: { title: 'Infra', kind: 'terraform', content: JSON.stringify({ 'main.tf': 'resource "x" {}', 'variables.tf': 'variable "x" {}' }) },
    });
    await flushPromises();
    expect(w.findAll('[data-testid="file-tab"]')).toHaveLength(2);
  });

  it('marks a file saved once its tool_result arrives', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: { title: 'deploy.sh', kind: 'bash', content: 'echo hi' } });
    await flushPromises();
    expect(w.find('[data-testid="file-tab-saving"]').exists()).toBe(true);
    ctl.onEvent({ type: 'tool_result', name: 'generate_artifact', preview: '{}' });
    await flushPromises();
    expect(w.find('[data-testid="file-tab-saved"]').exists()).toBe(true);
  });

  it('never shows a "how was this made" disclosure — the status line is the only window into activity now', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'thinking_delta', text: 'Planning the bundle...' });
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: { title: 'Infra' } });
    await flushPromises();
    expect(w.find('[data-testid="deploy-gen-details"]').exists()).toBe(false);
    expect(w.find('[data-testid="deploy-gen-activity"]').exists()).toBe(false);
  });

  it('walks the status line through each tool call as they run', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'tool_call', name: 'generate_artifact', input: { title: 'Infra' } });
    await flushPromises();
    expect(w.text()).toContain('Generating artifact Infra');
    ctl.onEvent({ type: 'tool_result', name: 'generate_artifact', preview: '{"id":"a1"}' });
    ctl.onEvent({ type: 'tool_call', name: 'set_execution_plan', input: {} });
    await flushPromises();
    expect(w.text()).toContain('Setting execution plan');
  });

  it('emits done when done event arrives', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'done' });
    await flushPromises();
    expect(w.emitted('done')).toBeTruthy();
  });

  it('shows error and stays mounted when error event arrives', async () => {
    let onEvent;
    api.generateProvisionStream.mockImplementation(async (_p, _d, cb) => {
      onEvent = cb;
      return new Promise(() => {});
    });
    const w = mountPanel();
    await flushPromises();
    onEvent({ type: 'error', message: 'LLM failed' });
    await flushPromises();
    expect(w.find('[data-testid="deploy-gen-error"]').exists()).toBe(true);
    expect(w.find('[data-testid="deploy-gen-error"]').text()).toContain('LLM failed');
    expect(w.emitted('done')).toBeFalsy();
  });

  it('shows error when the stream promise rejects', async () => {
    api.generateProvisionStream.mockRejectedValue(new Error('Network down'));
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="deploy-gen-error"]').exists()).toBe(true);
    expect(w.find('[data-testid="deploy-gen-error"]').text()).toContain('Network down');
    expect(w.emitted('done')).toBeFalsy();
  });

  it('shows a spinner while waiting for the first event', async () => {
    api.generateProvisionStream.mockImplementation(() => new Promise(() => {}));
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="deploy-gen-spinner"]').exists()).toBe(true);
  });

  it('Try again retries generation', async () => {
    const ctl = makeStreamController();
    const w = mountPanel();
    await flushPromises();
    ctl.onEvent({ type: 'error', message: 'LLM failed' });
    await flushPromises();
    await w.find('[data-testid="deploy-gen-back"]').exists();
    await w.find('[data-testid="deploy-gen-retry"]').trigger('click');
    await flushPromises();
    expect(api.generateProvisionStream).toHaveBeenCalledTimes(2);
    expect(w.find('[data-testid="deploy-gen-error"]').exists()).toBe(false);
  });
});

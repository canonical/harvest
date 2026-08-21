import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { nextTick } from 'vue';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getArtifact:            vi.fn(),
    generateProvision:      vi.fn(),
    proposeProvisionChange: vi.fn(),
    applyProvisionChange:   vi.fn(),
    deployDeployment:       vi.fn(),
    redeployDeployment:     vi.fn(),
    destroyDeployment:      vi.fn(),
    openProjectEvents:      vi.fn(),
    listProjectIssues:      vi.fn(),
  };
});

import ProvisionPhase from '../../src/components/deployment/ProvisionPhase.vue';
import * as api from '../../src/lib/api.js';

const AGENTS = [{ id: 'agent-1', hostname: 'host-1' }];
const AGENTS2 = [{ id: 'agent-1', hostname: 'host-1' }, { id: 'agent-2', hostname: 'host-2' }];

const NO_BUNDLE = { id: 'd1', infra_state: 'none', terraform_bundle: null };
const WITH_BUNDLE_NONE = { id: 'd1', infra_state: 'none', terraform_bundle: { id: 'a1', title: 'Infra', kind: 'terraform' } };
const WITH_BUNDLE_UP   = { id: 'd1', infra_state: 'up',   terraform_bundle: { id: 'a1', title: 'Infra', kind: 'terraform' } };
const WITH_BUNDLE_BROKEN = { id: 'd1', infra_state: 'broken', terraform_bundle: { id: 'a1', title: 'Infra', kind: 'terraform' } };

const FAILED_RUN = {
  id: 'r1', action: 'apply', status: 'failed', exit_code: 1,
  stdout_preview: '', stderr_preview: 'Error: connection refused',
  initiated_by: 'user', reasoning: null, created_at: '2026-08-07T12:00:00Z',
};

function mountPhase(deployment, runs = [], agents = AGENTS) {
  return mount(ProvisionPhase, {
    props: { projectId: 'proj-1', deployment, runs, agents },
    global: { stubs: { RouterLink: { props: ['to'], template: '<a :href="typeof to === \'string\' ? to : to.path"><slot /></a>' } } },
  });
}

describe('ProvisionPhase', () => {
  let capturedOnEvent;

  beforeEach(() => {
    vi.restoreAllMocks();
    capturedOnEvent = null;
    api.openProjectEvents.mockImplementation((projectId, convId, onEvent) => {
      capturedOnEvent = onEvent;
      return { close: vi.fn() };
    });
    api.listProjectIssues.mockResolvedValue([]);
  });

  it('with no bundle yet, automatically generates deployment artifacts, shows a busy state, and emits refresh', async () => {
    let resolveGenerate;
    api.generateProvision.mockReturnValue(new Promise(r => { resolveGenerate = r; }));
    const w = mountPhase(NO_BUNDLE);
    await nextTick();

    expect(api.generateProvision).toHaveBeenCalledWith('proj-1', 'd1');
    expect(w.text()).toContain('Generating deployment artifacts');

    resolveGenerate({});
    await flushPromises();
    expect(w.emitted('refresh')).toBeTruthy();
  });

  it('streams live status while generating deployment artifacts', async () => {
    let resolveGenerate;
    api.generateProvision.mockReturnValue(new Promise(r => { resolveGenerate = r; }));
    const w = mountPhase(NO_BUNDLE);
    await nextTick();

    expect(capturedOnEvent).toBeTypeOf('function');
    capturedOnEvent({ type: 'thinking', deployment_id: 'd1', text: 'Writing the Terraform bundle for this design' });
    capturedOnEvent({ type: 'tool_call', deployment_id: 'd1', name: 'generate_artifact', input: {} });
    capturedOnEvent({ type: 'tool_call', deployment_id: 'other-deployment', name: 'generate_artifact', input: {} });
    await nextTick();

    expect(w.text()).toContain('Writing the Terraform bundle for this design');
    expect(w.text()).toContain('Generating the Terraform/Terragrunt bundle');
    expect(w.find('[data-testid="generation-status"]').findAll('li')).toHaveLength(2);

    resolveGenerate({});
    await flushPromises();
  });

  it('shows a Retry button if automatic generation fails', async () => {
    api.generateProvision.mockRejectedValue(new Error('boom'));
    const w = mountPhase(NO_BUNDLE);
    await flushPromises();

    expect(w.find('[data-testid="generate-provision-btn"]').exists()).toBe(true);
    expect(w.text()).toContain('boom');
  });

  it('renders the bundle files on the right once a bundle exists', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: JSON.stringify({ 'main.tf': 'resource "x" {}' }) });
    const w = mountPhase(WITH_BUNDLE_NONE);
    await flushPromises();
    expect(w.text()).toContain('main.tf');
    expect(w.text()).toContain('resource "x" {}');
  });

  it('shows an enabled Deploy button (not a chat) when infra_state is none, with a single agent pre-selected; Redeploy/Destroy are disabled', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const w = mountPhase(WITH_BUNDLE_NONE);
    await flushPromises();

    expect(w.find('[data-testid="deploy-btn"]').exists()).toBe(true);
    expect(w.find('[data-testid="redeploy-btn"]').exists()).toBe(true);
    expect(w.find('[data-testid="broken-issues-banner"]').exists()).toBe(false);
    expect(w.find('[data-testid="deploy-btn"]').attributes('disabled')).toBeUndefined();
    expect(w.find('[data-testid="redeploy-btn"]').attributes('disabled')).toBeDefined();
    expect(w.find('[data-testid="destroy-btn"]').attributes('disabled')).toBeDefined();
  });

  it('clicking Deploy calls deployDeployment with the selected agent and streams to Run History', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    let resolveDeploy;
    api.deployDeployment.mockReturnValue(new Promise(r => { resolveDeploy = r; }));
    const w = mountPhase(WITH_BUNDLE_NONE);
    await flushPromises();

    await w.find('[data-testid="deploy-btn"]').trigger('click');
    await nextTick();

    expect(w.find('[data-testid="run-history-tab"]').classes()).toContain('provision-tab--active');
    expect(w.text()).toContain('Deploying on host-1');
    expect(api.deployDeployment).toHaveBeenCalledWith('proj-1', 'd1', { agent_id: 'agent-1' });

    resolveDeploy({ runs: [{ action: 'apply', exit_code: 0 }] });
    await flushPromises();
    expect(w.emitted('refresh')).toBeTruthy();
  });

  it('requires picking an agent before Deploy is enabled when there are multiple agents', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const w = mountPhase(WITH_BUNDLE_NONE, [], AGENTS2);
    await flushPromises();

    expect(w.find('[data-testid="deploy-btn"]').attributes('disabled')).toBeDefined();
    await w.find('[data-testid="provision-agent-select"]').setValue('agent-2');
    expect(w.find('[data-testid="deploy-btn"]').attributes('disabled')).toBeUndefined();
  });

  it('shows an enabled Redeploy and Destroy, and a disabled Deploy, when infra_state is up', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const w = mountPhase(WITH_BUNDLE_UP);
    await flushPromises();

    expect(w.find('[data-testid="deploy-btn"]').attributes('disabled')).toBeDefined();
    expect(w.find('[data-testid="redeploy-btn"]').attributes('disabled')).toBeUndefined();
    expect(w.find('[data-testid="destroy-btn"]').attributes('disabled')).toBeUndefined();
  });

  it('clicking Redeploy calls redeployDeployment', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    api.redeployDeployment.mockResolvedValue({ runs: [{ action: 'apply', exit_code: 0 }] });
    const w = mountPhase(WITH_BUNDLE_UP);
    await flushPromises();

    await w.find('[data-testid="redeploy-btn"]').trigger('click');
    await flushPromises();

    expect(api.redeployDeployment).toHaveBeenCalledWith('proj-1', 'd1', { agent_id: 'agent-1' });
  });

  it('Request a change is available when infra_state is up', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: JSON.stringify({ 'main.tf': 'a' }) });
    api.proposeProvisionChange.mockResolvedValue({
      explanation: 'Added a second VM.',
      current_files: { 'main.tf': 'a' },
      proposed_files: { 'main.tf': 'a\nb' },
    });
    const w = mountPhase(WITH_BUNDLE_UP);
    await flushPromises();

    await w.find('[data-testid="propose-change-instructions"]').setValue('add a second VM');
    await w.find('[data-testid="propose-change-btn"]').trigger('click');
    await flushPromises();

    expect(api.proposeProvisionChange).toHaveBeenCalledWith('proj-1', 'd1', { instructions: 'add a second VM' });
    expect(w.find('.diff-view').exists()).toBe(true);
  });

  it('Apply calls applyProvisionChange with the proposed files and clears the pending change', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: JSON.stringify({ 'main.tf': 'a' }) });
    api.proposeProvisionChange.mockResolvedValue({
      explanation: 'x', current_files: { 'main.tf': 'a' }, proposed_files: { 'main.tf': 'b' },
    });
    api.applyProvisionChange.mockResolvedValue({});
    const w = mountPhase(WITH_BUNDLE_UP);
    await flushPromises();
    await w.find('[data-testid="propose-change-instructions"]').setValue('note');
    await w.find('[data-testid="propose-change-btn"]').trigger('click');
    await flushPromises();

    await w.find('[data-testid="approve-change-btn"]').trigger('click');
    await flushPromises();

    expect(api.applyProvisionChange).toHaveBeenCalledWith('proj-1', 'd1', { files: { 'main.tf': 'b' } });
    expect(w.find('.diff-view').exists()).toBe(false);
    expect(w.emitted('refresh')).toBeTruthy();
  });

  it('keeps Redeploy/Destroy usable in the toolbar while a diff is displayed', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: JSON.stringify({ 'main.tf': 'a' }) });
    api.proposeProvisionChange.mockResolvedValue({
      explanation: 'x', current_files: { 'main.tf': 'a' }, proposed_files: { 'main.tf': 'b' },
    });
    const w = mountPhase(WITH_BUNDLE_UP);
    await flushPromises();
    await w.find('[data-testid="propose-change-instructions"]').setValue('note');
    await w.find('[data-testid="propose-change-btn"]').trigger('click');
    await flushPromises();

    expect(w.find('.diff-view').exists()).toBe(true);
    expect(w.find('[data-testid="redeploy-btn"]').exists()).toBe(true);
    expect(w.find('[data-testid="redeploy-btn"]').attributes('disabled')).toBeUndefined();
    expect(w.find('[data-testid="destroy-btn"]').attributes('disabled')).toBeUndefined();
  });

  it('Apply does not auto-redeploy', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: JSON.stringify({ 'main.tf': 'a' }) });
    api.proposeProvisionChange.mockResolvedValue({
      explanation: 'x', current_files: { 'main.tf': 'a' }, proposed_files: { 'main.tf': 'b' },
    });
    api.applyProvisionChange.mockResolvedValue({});
    const w = mountPhase(WITH_BUNDLE_UP);
    await flushPromises();
    await w.find('[data-testid="propose-change-instructions"]').setValue('note');
    await w.find('[data-testid="propose-change-btn"]').trigger('click');
    await flushPromises();
    await w.find('[data-testid="approve-change-btn"]').trigger('click');
    await flushPromises();

    expect(api.redeployDeployment).not.toHaveBeenCalled();
  });

  it('Discard clears the pending change without applying it', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: JSON.stringify({ 'main.tf': 'a' }) });
    api.proposeProvisionChange.mockResolvedValue({
      explanation: 'x', current_files: { 'main.tf': 'a' }, proposed_files: { 'main.tf': 'b' },
    });
    const w = mountPhase(WITH_BUNDLE_UP);
    await flushPromises();
    await w.find('[data-testid="propose-change-instructions"]').setValue('note');
    await w.find('[data-testid="propose-change-btn"]').trigger('click');
    await flushPromises();

    await w.find('[data-testid="discard-change-btn"]').trigger('click');
    await flushPromises();

    expect(api.applyProvisionChange).not.toHaveBeenCalled();
    expect(w.find('.diff-view').exists()).toBe(false);
  });

  it('shows a banner linking to Issues when the deployment is broken, with an open-issue count', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    api.listProjectIssues.mockResolvedValue([
      { id: 'i1', status: 'untriaged' },
      { id: 'i2', status: 'in_progress' },
      { id: 'i3', status: 'fixed' },
    ]);
    const w = mountPhase(WITH_BUNDLE_BROKEN, [FAILED_RUN]);
    await flushPromises();

    expect(api.listProjectIssues).toHaveBeenCalledWith('proj-1', { deploymentId: 'd1' });
    const banner = w.find('[data-testid="broken-issues-banner"]');
    expect(banner.exists()).toBe(true);
    expect(banner.text()).toContain('2 open issues');
    const link = w.find('[data-testid="view-issues-link"]');
    expect(link.attributes('href')).toContain('/issues?deployment=d1');
  });

  it('does not show the broken-issues banner when infra_state is not broken', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const w = mountPhase(WITH_BUNDLE_UP);
    await flushPromises();

    expect(w.find('[data-testid="broken-issues-banner"]').exists()).toBe(false);
    expect(api.listProjectIssues).not.toHaveBeenCalled();
  });

  it('streams live terraform output lines for the running deployment into the Run History tab', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    let resolveDeploy;
    api.deployDeployment.mockReturnValue(new Promise(r => { resolveDeploy = r; }));
    const w = mountPhase(WITH_BUNDLE_NONE);
    await flushPromises();

    await w.find('[data-testid="deploy-btn"]').trigger('click');
    await nextTick();

    expect(capturedOnEvent).toBeTypeOf('function');
    capturedOnEvent({ type: 'deployment_run_log', deployment_id: 'd1', stream: 'stdout', line: 'Initializing the backend...' });
    capturedOnEvent({ type: 'deployment_run_log', deployment_id: 'other-deployment', stream: 'stdout', line: 'should be ignored' });
    await nextTick();

    expect(w.text()).toContain('Initializing the backend...');
    expect(w.text()).not.toContain('should be ignored');

    resolveDeploy({ runs: [{ action: 'apply', exit_code: 0 }] });
    await flushPromises();
  });

  it('closes its project event subscription on unmount', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const close = vi.fn();
    api.openProjectEvents.mockReturnValue({ close });
    const w = mountPhase(WITH_BUNDLE_NONE);
    await flushPromises();

    w.unmount();
    expect(close).toHaveBeenCalled();
  });

  it('shows a past failure from run history on a fresh mount when infra_state is not broken', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const w = mountPhase(WITH_BUNDLE_NONE, [FAILED_RUN]);
    await flushPromises();

    expect(api.proposeProvisionChange).not.toHaveBeenCalled();
    await w.find('[data-testid="run-history-tab"]').trigger('click');

    expect(w.find('[data-testid="run-history"]').exists()).toBe(true);
    expect(w.text()).toContain('Error: connection refused');
  });
});

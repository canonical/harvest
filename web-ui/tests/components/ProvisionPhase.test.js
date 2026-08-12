import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { nextTick } from 'vue';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getArtifact:              vi.fn(),
    generateProvision:        vi.fn(),
    proposeProvisionChange:   vi.fn(),
    applyProvisionChange:     vi.fn(),
    deployDeployment:         vi.fn(),
    redeployDeployment:       vi.fn(),
    destroyDeployment:        vi.fn(),
    openProjectEvents:        vi.fn(),
    diagnoseProvisionFailure: vi.fn(),
    dismissProvisionDiagnosis: vi.fn(),
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
    api.diagnoseProvisionFailure.mockResolvedValue({ started: true });
    api.dismissProvisionDiagnosis.mockResolvedValue({});
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
    expect(w.find('[data-testid="diagnosis-trace"]').exists()).toBe(false);
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

  it('Apply does not auto-redeploy when infra was healthy (not broken) beforehand', async () => {
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

  it('on mount in a broken state with a failed run, auto-triggers diagnosis instead of calling proposeProvisionChange', async () => {
    let resolveDiagnose;
    api.diagnoseProvisionFailure.mockReturnValue(new Promise(r => { resolveDiagnose = r; }));
    api.getArtifact.mockResolvedValue({ id: 'a1', content: JSON.stringify({ 'main.tf': 'a' }) });
    const w = mountPhase(WITH_BUNDLE_BROKEN, [FAILED_RUN]);
    await nextTick();

    expect(api.proposeProvisionChange).not.toHaveBeenCalled();
    expect(api.diagnoseProvisionFailure).toHaveBeenCalledWith('proj-1', 'd1');
    expect(w.text()).toContain('Diagnosing');

    resolveDiagnose({ started: true });
    await flushPromises();
  });

  it('does not auto-trigger diagnosis when there is no failed run', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    mountPhase(WITH_BUNDLE_BROKEN, []);
    await flushPromises();

    expect(api.diagnoseProvisionFailure).not.toHaveBeenCalled();
  });

  it('does not re-trigger diagnosis for a failed run that already has a running diagnosis', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const deployment = { ...WITH_BUNDLE_BROKEN, diagnosis: { status: 'running', run_id: 'r1' } };
    mountPhase(deployment, [FAILED_RUN]);
    await flushPromises();

    expect(api.diagnoseProvisionFailure).not.toHaveBeenCalled();
  });

  it('streams live diagnosis status while diagnosing, with Redeploy/Destroy still available', async () => {
    let resolveDiagnose;
    api.diagnoseProvisionFailure.mockReturnValue(new Promise(r => { resolveDiagnose = r; }));
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const w = mountPhase(WITH_BUNDLE_BROKEN, [FAILED_RUN]);
    await flushPromises();

    expect(capturedOnEvent).toBeTypeOf('function');
    capturedOnEvent({ type: 'thinking', deployment_id: 'd1', text: 'Reading the failure logs' });
    capturedOnEvent({ type: 'tool_call', deployment_id: 'd1', name: 'read_provision_bundle', input: {} });
    capturedOnEvent({ type: 'tool_call', deployment_id: 'other-deployment', name: 'read_provision_bundle', input: {} });
    await nextTick();

    expect(w.text()).toContain('Reading the failure logs');
    expect(w.find('[data-testid="diagnosis-trace"]').findAll('li')).toHaveLength(2);
    expect(w.find('[data-testid="deploy-btn"]').attributes('disabled')).toBeDefined();
    expect(w.find('[data-testid="redeploy-btn"]').attributes('disabled')).toBeUndefined();
    expect(w.find('[data-testid="destroy-btn"]').attributes('disabled')).toBeUndefined();

    resolveDiagnose({ started: true });
    await flushPromises();
  });

  it('shows the diagnosis diff once proposed, without a chat or free-text input', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const deployment = {
      ...WITH_BUNDLE_BROKEN,
      diagnosis: { status: 'proposed', run_id: 'r1', explanation: 'The security group blocked the health check.', files: { 'main.tf': 'fixed' } },
    };
    const w = mountPhase(deployment, [FAILED_RUN]);
    await flushPromises();

    expect(api.diagnoseProvisionFailure).not.toHaveBeenCalled();
    expect(w.text()).toContain('The security group blocked the health check.');
    expect(w.find('.diff-view').exists()).toBe(true);
    expect(w.find('[data-testid="approve-change-btn"]').exists()).toBe(true);
    expect(w.find('textarea').exists()).toBe(false);
  });

  it('Discard on a proposed diagnosis also dismisses it on the server', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const deployment = {
      ...WITH_BUNDLE_BROKEN,
      diagnosis: { status: 'proposed', run_id: 'r1', explanation: 'x', files: { 'main.tf': 'fixed' } },
    };
    const w = mountPhase(deployment, [FAILED_RUN]);
    await flushPromises();

    await w.find('[data-testid="discard-change-btn"]').trigger('click');
    await flushPromises();

    expect(api.dismissProvisionDiagnosis).toHaveBeenCalledWith('proj-1', 'd1');
    expect(w.emitted('refresh')).toBeTruthy();
  });

  it('Apply auto-redeploys with the selected agent once infra was broken', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    api.applyProvisionChange.mockResolvedValue({});
    api.redeployDeployment.mockResolvedValue({ runs: [{ action: 'apply', exit_code: 0 }] });
    const deployment = {
      ...WITH_BUNDLE_BROKEN,
      diagnosis: { status: 'proposed', run_id: 'r1', explanation: 'x', files: { 'main.tf': 'fixed' } },
    };
    const w = mountPhase(deployment, [FAILED_RUN]);
    await flushPromises();

    await w.find('[data-testid="approve-change-btn"]').trigger('click');
    await flushPromises();

    expect(api.applyProvisionChange).toHaveBeenCalledWith('proj-1', 'd1', { files: { 'main.tf': 'fixed' } });
    expect(api.redeployDeployment).toHaveBeenCalledWith('proj-1', 'd1', { agent_id: 'agent-1' });
  });

  it('shows a failed diagnosis with a Retry button, which re-triggers diagnosis', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const deployment = { ...WITH_BUNDLE_BROKEN, diagnosis: { status: 'failed', run_id: 'r1', error: 'agent unreachable' } };
    const w = mountPhase(deployment, [FAILED_RUN]);
    await flushPromises();

    expect(api.diagnoseProvisionFailure).not.toHaveBeenCalled();
    expect(w.text()).toContain('agent unreachable');

    await w.find('[data-testid="retry-diagnosis-btn"]').trigger('click');
    await flushPromises();

    expect(api.diagnoseProvisionFailure).toHaveBeenCalledWith('proj-1', 'd1');
  });

  it('a done event for this deployment refreshes so the diagnosis result is picked up', async () => {
    let resolveDiagnose;
    api.diagnoseProvisionFailure.mockReturnValue(new Promise(r => { resolveDiagnose = r; }));
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const w = mountPhase(WITH_BUNDLE_BROKEN, [FAILED_RUN]);
    await flushPromises();

    capturedOnEvent({ type: 'done', deployment_id: 'd1', answer: '' });
    await nextTick();

    expect(w.emitted('refresh')).toBeTruthy();
    resolveDiagnose({ started: true });
    await flushPromises();
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

  it('shows a past failure from run history on a fresh mount when infra_state is not broken, with no auto-diagnosis', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '{}' });
    const w = mountPhase(WITH_BUNDLE_NONE, [FAILED_RUN]);
    await flushPromises();

    expect(api.proposeProvisionChange).not.toHaveBeenCalled();
    await w.find('[data-testid="run-history-tab"]').trigger('click');

    expect(w.find('[data-testid="run-history"]').exists()).toBe(true);
    expect(w.text()).toContain('Error: connection refused');
  });
});

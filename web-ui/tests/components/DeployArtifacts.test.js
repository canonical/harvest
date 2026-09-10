import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getExecutionPlan: vi.fn(),
    setExecutionPlan: vi.fn(),
    openProjectEvents: vi.fn(() => ({ close() {} })),
    proposeProvisionChangeStream: vi.fn(),
    applyProvisionChange: vi.fn(),
  };
});

vi.mock('../../src/components/deployment/ArtifactEditor.vue', () => ({
  default: {
    name: 'ArtifactEditor',
    template: `<div data-testid="artifact-editor" :data-artifact-id="String(artifactId)">
      <div v-if="bashPair" class="artifact-editor__tabs" data-testid="script-tabs">
        <button data-testid="script-tab-deploy" @click="$emit('script-tab-change', 'deploy')">Deploy</button>
        <button data-testid="script-tab-destroy" @click="$emit('script-tab-change', 'destroy')">Destroy</button>
      </div>
    </div>`,
    props: ['projectId', 'deploymentId', 'artifactId', 'bashPair', 'scriptTab'],
    emits: ['saved', 'script-tab-change'],
  },
}));

vi.mock('../../src/components/deployment/AddArtifactModal.vue', () => ({
  default: {
    name: 'AddArtifactModal',
    template: '<div v-if="open" data-testid="add-artifact-modal-stub" @click="$emit(\'close\')" />',
    props: ['open', 'projectId', 'deploymentId'],
    emits: ['close', 'added'],
  },
}));

vi.mock('../../src/components/deployment/DesignGenerationPanel.vue', () => ({
  default: {
    name: 'DesignGenerationPanel',
    template: '<div data-testid="design-generation-stub" />',
    props: ['projectId', 'deploymentId', 'streamFn', 'body', 'preparingText', 'readyText', 'failedText'],
    emits: ['done', 'cancel'],
  },
}));

vi.mock('../../src/components/deployment/ProposalReview.vue', () => ({
  default: {
    name: 'ProposalReview',
    template: '<div data-testid="proposal-review-stub" />',
    props: ['projectId', 'deploymentId', 'proposedFiles', 'originalFiles', 'executionPlan'],
    emits: ['apply', 'discard', 'modify'],
  },
}));

import DeployArtifacts from '../../src/components/deployment/DeployArtifacts.vue';
import * as api from '../../src/lib/api.js';

const DEPLOYMENT = {
  id: 'd1', name: 'MyProject', infra_state: 'none',
  terraform_bundle: { id: 'b1', kind: 'terraform' },
  context_artifacts: [],
};

const PLAN = {
  deploy_steps: [
    { id: 's0', action: 'run', label: 'Prep', artifact: { id: 'a0', kind: 'bash', title: 'deploy-prep.sh' }, depends_on: [] },
    { id: 's1', action: 'apply', label: 'Apply', artifact: { id: 'a1', kind: 'terraform', title: 'Infra' }, depends_on: ['s0'] },
  ],
  destroy_steps: [
    { id: 's2', action: 'destroy', label: 'Destroy infra', artifact: { id: 'a1', kind: 'terraform', title: 'Infra' }, depends_on: [] },
    { id: 's3', action: 'destroy', label: 'Teardown prep', artifact: { id: 'a2', kind: 'bash', title: 'destroy-prep.sh' }, depends_on: ['s2'] },
  ],
};

function mountPanel({ deployment = DEPLOYMENT, agents = [] } = {}) {
  return mount(DeployArtifacts, {
    props: { projectId: 'proj-1', deployment, agents },
  });
}

describe('DeployArtifacts', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.getExecutionPlan.mockResolvedValue(PLAN);
    api.setExecutionPlan.mockResolvedValue({});
    api.applyProvisionChange.mockResolvedValue({});
  });

  it('renders the sidebar and editor', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="deploy-artifacts-sidebar"]').exists()).toBe(true);
    expect(w.find('[data-testid="artifact-editor"]').exists()).toBe(true);
  });

  it('loads the execution plan on mount', async () => {
    mountPanel();
    await flushPromises();
    expect(api.getExecutionPlan).toHaveBeenCalledWith('proj-1', 'd1');
  });

  it('renders the pipeline stepper with steps in topological order', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="pipeline-stepper"]').exists()).toBe(true);
    const steps = w.findAll('.pipeline-stepper__item');
    expect(steps).toHaveLength(2);
    expect(w.find('[data-testid="pipeline-step-a0"]').exists()).toBe(true);
    expect(w.find('[data-testid="pipeline-step-a1"]').exists()).toBe(true);
  });

  it('shows dependency info in the pipeline cards', async () => {
    const w = mountPanel();
    await flushPromises();
    const stepA1 = w.find('[data-testid="pipeline-step-a1"]');
    expect(stepA1.text()).toContain('depends on');
    expect(stepA1.text()).toContain('Infra');
  });

  it('shows deploy and destroy phase tabs', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="phase-tab-deploy"]').exists()).toBe(true);
    expect(w.find('[data-testid="phase-tab-destroy"]').exists()).toBe(true);
  });

  it('switches to destroy steps when destroy phase tab is clicked', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="phase-tab-destroy"]').trigger('click');
    await flushPromises();
    const steps = w.findAll('.pipeline-stepper__item');
    expect(steps).toHaveLength(2);
    expect(w.find('[data-testid="pipeline-step-a2"]').exists()).toBe(true);
  });

  it('selects the artifact when a pipeline step is clicked', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="pipeline-step-a1"]').trigger('click');
    await flushPromises();
    const editors = w.findAll('[data-testid="artifact-editor"]');
    const visible = editors.filter(e => e.isVisible());
    expect(visible).toHaveLength(1);
    expect(visible[0].attributes('data-artifact-id')).toBe('a1');
  });

  it('shows deploy and destroy script tabs when a bash pair step is selected', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="pipeline-step-a0"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="script-tab-deploy"]').exists()).toBe(true);
    expect(w.find('[data-testid="script-tab-destroy"]').exists()).toBe(true);
  });

  it('shows the deploy script artifact in the editor by default for bash', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="pipeline-step-a0"]').trigger('click');
    await flushPromises();
    const editors = w.findAll('[data-testid="artifact-editor"]');
    const visible = editors.filter(e => e.isVisible());
    expect(visible).toHaveLength(1);
    expect(visible[0].attributes('data-artifact-id')).toBe('a0');
  });

  it('switches to the destroy script artifact when destroy tab is clicked', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="pipeline-step-a0"]').trigger('click');
    await flushPromises();
    await w.find('[data-testid="script-tab-destroy"]').trigger('click');
    await flushPromises();
    const editors = w.findAll('[data-testid="artifact-editor"]');
    const visible = editors.filter(e => e.isVisible());
    expect(visible).toHaveLength(1);
    expect(visible[0].attributes('data-artifact-id')).toBe('a2');
  });

  it('does not show script tabs when a terraform artifact is selected', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="pipeline-step-a1"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="script-tab-deploy"]').exists()).toBe(false);
    expect(w.find('[data-testid="script-tab-destroy"]').exists()).toBe(false);
  });

  it('passes projectId and deploymentId to the ArtifactEditor', async () => {
    const w = mountPanel();
    await flushPromises();
    const editor = w.findComponent({ name: 'ArtifactEditor' });
    expect(editor.props('projectId')).toBe('proj-1');
    expect(editor.props('deploymentId')).toBe('d1');
  });

  it('shows no artifact selected initially', async () => {
    const w = mountPanel();
    await flushPromises();
    const editors = w.findAll('[data-testid="artifact-editor"]');
    const visible = editors.filter(e => e.isVisible());
    expect(visible).toHaveLength(1);
    expect(visible[0].attributes('data-artifact-id')).toBe('null');
  });

  it('shows the infra-state badge', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('.infra-state-badge').exists()).toBe(true);
  });

  it('shows an Add artifact button', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="add-artifact-btn"]').exists()).toBe(true);
  });

  it('shows a global Propose a change button', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="propose-change-btn"]').exists()).toBe(true);
  });

  it('opens the propose modal when Propose a change is clicked', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="propose-change-btn"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="propose-change-modal"]').exists()).toBe(true);
  });

  it('closes the propose modal on cancel', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="propose-change-btn"]').trigger('click');
    await flushPromises();
    await w.find('[data-testid="propose-change-modal"]').find('.modal-close').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="propose-change-modal"]').exists()).toBe(false);
  });

  it('submits the proposal and shows the generation panel', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="propose-change-btn"]').trigger('click');
    await flushPromises();
    await w.find('[data-testid="propose-change-prompt"]').setValue('Add a new output variable');
    await w.find('[data-testid="submit-propose-change-btn"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="design-generation-stub"]').exists()).toBe(true);
  });

  it('shows proposal review when generation emits done with proposed files', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="propose-change-btn"]').trigger('click');
    await flushPromises();
    await w.find('[data-testid="propose-change-prompt"]').setValue('Add a new output variable');
    await w.find('[data-testid="submit-propose-change-btn"]').trigger('click');
    await flushPromises();
    const proposed = { 'main.tf': '# new terraform', 'deploy-setup.sh': '#!/bin/bash\necho hi' };
    w.findComponent({ name: 'DesignGenerationPanel' }).vm.$emit('done', {
      answer: 'Here are the changes\n```json\n' + JSON.stringify(proposed) + '\n```',
      text: '',
    });
    await flushPromises();
    expect(w.find('[data-testid="proposal-review-stub"]').exists()).toBe(true);
  });

  it('applies the proposal when ProposalReview emits apply', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="propose-change-btn"]').trigger('click');
    await flushPromises();
    await w.find('[data-testid="propose-change-prompt"]').setValue('Add a new output variable');
    await w.find('[data-testid="submit-propose-change-btn"]').trigger('click');
    await flushPromises();
    const proposed = { 'main.tf': '# new terraform' };
    w.findComponent({ name: 'DesignGenerationPanel' }).vm.$emit('done', {
      answer: '```json\n' + JSON.stringify(proposed) + '\n```',
      text: '',
    });
    await flushPromises();
    const filesToApply = { 'main.tf': '# new terraform' };
    w.findComponent({ name: 'ProposalReview' }).vm.$emit('apply', filesToApply);
    await flushPromises();
    expect(api.applyProvisionChange).toHaveBeenCalledWith('proj-1', 'd1', { files: filesToApply });
    expect(w.find('[data-testid="deploy-artifacts-sidebar"]').exists()).toBe(true);
  });

  it('discards the proposal and returns to the editor view', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="propose-change-btn"]').trigger('click');
    await flushPromises();
    await w.find('[data-testid="propose-change-prompt"]').setValue('Add a new output variable');
    await w.find('[data-testid="submit-propose-change-btn"]').trigger('click');
    await flushPromises();
    const proposed = { 'main.tf': '# new terraform' };
    w.findComponent({ name: 'DesignGenerationPanel' }).vm.$emit('done', {
      answer: '```json\n' + JSON.stringify(proposed) + '\n```',
      text: '',
    });
    await flushPromises();
    w.findComponent({ name: 'ProposalReview' }).vm.$emit('discard');
    await flushPromises();
    expect(w.find('[data-testid="proposal-review-stub"]').exists()).toBe(false);
    expect(w.find('[data-testid="deploy-artifacts-sidebar"]').exists()).toBe(true);
  });

  it('opens the AddArtifactModal when Add artifact is clicked', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="add-artifact-btn"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="add-artifact-modal-stub"]').exists()).toBe(true);
  });

  it('closes the AddArtifactModal on close event', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="add-artifact-btn"]').trigger('click');
    await flushPromises();
    await w.find('[data-testid="add-artifact-modal-stub"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="add-artifact-modal-stub"]').exists()).toBe(false);
  });

  it('adds the new artifact to the execution plan and reloads on added event', async () => {
    api.getExecutionPlan.mockResolvedValueOnce(PLAN);
    const w = mountPanel();
    await flushPromises();
    const newArtifact = { id: 'a3', kind: 'bash', content: '#!/bin/bash\necho hi' };
    w.findComponent({ name: 'AddArtifactModal' }).vm.$emit('added', newArtifact);
    await flushPromises();
    expect(api.setExecutionPlan).toHaveBeenCalledWith('proj-1', 'd1', expect.objectContaining({
      deploy_steps: expect.arrayContaining([
        expect.objectContaining({ artifact_id: 'a3', action: 'run' }),
      ]),
    }));
    expect(api.getExecutionPlan).toHaveBeenCalledTimes(2);
    const editors = w.findAll('[data-testid="artifact-editor"]');
    const visible = editors.filter(e => e.isVisible());
    expect(visible).toHaveLength(1);
    expect(visible[0].attributes('data-artifact-id')).toBe('a3');
  });

  it('does not emit refresh when ArtifactEditor emits saved', async () => {
    const w = mountPanel();
    await flushPromises();
    w.findComponent({ name: 'ArtifactEditor' }).vm.$emit('saved');
    await flushPromises();
    expect(w.emitted('refresh')).toBeFalsy();
  });
});

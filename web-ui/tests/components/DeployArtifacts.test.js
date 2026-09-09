import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getExecutionPlan: vi.fn(),
    setExecutionPlan: vi.fn(),
    openProjectEvents: vi.fn(() => ({ close() {} })),
  };
});

vi.mock('../../src/components/deployment/ArtifactEditor.vue', () => ({
  default: {
    name: 'ArtifactEditor',
    template: '<div data-testid="artifact-editor" />',
    props: ['projectId', 'deploymentId', 'artifactId'],
    emits: ['saved'],
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

import DeployArtifacts from '../../src/components/deployment/DeployArtifacts.vue';
import * as api from '../../src/lib/api.js';

const DEPLOYMENT = {
  id: 'd1', name: 'MyProject', infra_state: 'none',
  terraform_bundle: { id: 'b1', kind: 'terraform' },
  context_artifacts: [],
};

const PLAN = {
  deploy_steps: [
    { id: 's0', action: 'run', label: 'Prep', artifact: { id: 'a0', kind: 'bash', title: 'Prep' }, depends_on: [] },
    { id: 's1', action: 'apply', label: 'Apply', artifact: { id: 'a1', kind: 'terraform', title: 'Infra' }, depends_on: ['s0'] },
  ],
  destroy_steps: [],
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

  it('lists unique artifacts from the execution plan in the sidebar', async () => {
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="artifact-item-a0"]').exists()).toBe(true);
    expect(w.find('[data-testid="artifact-item-a1"]').exists()).toBe(true);
  });

  it('selects the artifact when a sidebar item is clicked', async () => {
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="artifact-item-a1"]').trigger('click');
    await flushPromises();
    expect(w.findComponent({ name: 'ArtifactEditor' }).props('artifactId')).toBe('a1');
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
    expect(w.findComponent({ name: 'ArtifactEditor' }).props('artifactId')).toBeNull();
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
    const newArtifact = { id: 'a2', kind: 'bash', content: '#!/bin/bash\necho hi' };
    w.findComponent({ name: 'AddArtifactModal' }).vm.$emit('added', newArtifact);
    await flushPromises();
    expect(api.setExecutionPlan).toHaveBeenCalledWith('proj-1', 'd1', expect.objectContaining({
      deploy_steps: expect.arrayContaining([
        expect.objectContaining({ artifact_id: 'a2', action: 'run' }),
      ]),
    }));
    expect(api.getExecutionPlan).toHaveBeenCalledTimes(2);
    expect(w.findComponent({ name: 'ArtifactEditor' }).props('artifactId')).toBe('a2');
  });

  it('does not emit refresh when ArtifactEditor emits saved', async () => {
    const w = mountPanel();
    await flushPromises();
    w.findComponent({ name: 'ArtifactEditor' }).vm.$emit('saved');
    await flushPromises();
    expect(w.emitted('refresh')).toBeFalsy();
  });
});

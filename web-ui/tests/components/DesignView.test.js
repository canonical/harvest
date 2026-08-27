import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { useProjectStore } from '../../src/stores/project.js';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getProjectDeploymentSingle: vi.fn(),
    listProjectArtifacts:       vi.fn(() => Promise.resolve([])),
    listTemplates:             vi.fn(() => Promise.resolve([])),
    generateDesign:             vi.fn(() => Promise.resolve({})),
    generateDesignStream:       vi.fn(() => new Promise(() => {})),
  };
});

import DesignView from '../../src/views/DesignView.vue';
import * as api from '../../src/lib/api.js';

const DEPLOYMENT_NO_DESIGN = {
  id: 'd1', name: 'MyProject', infra_state: 'none', design_doc: null,
};

const DEPLOYMENT_WITH_DESIGN = {
  id: 'd1', name: 'MyProject', infra_state: 'none',
  design_doc: { id: 'a1', title: 'Design doc' },
};

const DEPLOYMENT_WITH_TEMPLATE = {
  id: 'd1', name: 'MyProject', infra_state: 'none',
  design_doc: { id: 'a1', title: 'Design doc' },
  template: { id: 't1', name: 'Gateway' },
};

let pinia;
function mountView() {
  return mount(DesignView, {
    props: { projectId: 'proj-1' },
    global: { plugins: [pinia] },
  });
}

function seedSelectedProject() {
  const store = useProjectStore();
  store.selectedProject = { id: 'proj-1', name: 'MyProject', group_id: 'g1' };
}

describe('DesignView', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    pinia = createPinia();
    setActivePinia(pinia);
  });

  it('fetches the project deployment on mount', async () => {
    seedSelectedProject();
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_NO_DESIGN));
    const w = mountView();
    await flushPromises();
    expect(api.getProjectDeploymentSingle).toHaveBeenCalledWith('proj-1');
  });

  it('shows the deployment name in the header for the setup state', async () => {
    seedSelectedProject();
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_NO_DESIGN));
    const w = mountView();
    await flushPromises();
    expect(w.text()).toContain('MyProject');
  });

  it('shows a Design eyebrow label in the header for the setup state', async () => {
    seedSelectedProject();
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_NO_DESIGN));
    const w = mountView();
    await flushPromises();
    expect(w.find('[data-testid="design-eyebrow"]').exists()).toBe(true);
    expect(w.find('[data-testid="design-eyebrow"]').text()).toMatch(/design/i);
  });

  it('shows a template chip in the header when the deployment uses a template', async () => {
    seedSelectedProject();
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_WITH_TEMPLATE));
    const w = mountView();
    await flushPromises();
    const chip = w.find('[data-testid="design-template-chip"]');
    expect(chip.exists()).toBe(true);
    expect(chip.text()).toContain('Gateway');
  });

  it('does not show a template chip when no template is linked', async () => {
    seedSelectedProject();
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_WITH_DESIGN));
    const w = mountView();
    await flushPromises();
    expect(w.find('[data-testid="design-template-chip"]').exists()).toBe(false);
  });

  it('renders the DesignPanel when a design doc exists', async () => {
    seedSelectedProject();
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_WITH_DESIGN));
    const w = mountView();
    await flushPromises();
    expect(w.findComponent({ name: 'DesignPanel' }).exists()).toBe(true);
    expect(w.findComponent({ name: 'DesignSetupPanel' }).exists()).toBe(false);
  });

  it('renders the DesignSetupPanel when no design doc exists yet', async () => {
    seedSelectedProject();
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_NO_DESIGN));
    const w = mountView();
    await flushPromises();
    expect(w.findComponent({ name: 'DesignSetupPanel' }).exists()).toBe(true);
    expect(w.findComponent({ name: 'DesignPanel' }).exists()).toBe(false);
  });

  it('passes the deployment id and group id to the setup panel', async () => {
    seedSelectedProject();
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_NO_DESIGN));
    const w = mountView();
    await flushPromises();
    const panel = w.findComponent({ name: 'DesignSetupPanel' });
    expect(panel.props('deploymentId')).toBe('d1');
    expect(panel.props('projectId')).toBe('proj-1');
    expect(panel.props('groupId')).toBe('g1');
  });

  it('shows a loading indicator while fetching', async () => {
    seedSelectedProject();
    let resolveFn;
    api.getProjectDeploymentSingle.mockReturnValue(new Promise(r => { resolveFn = r; }));
    const w = mountView();
    await flushPromises();
    expect(w.find('[data-testid="design-loading"]').exists()).toBe(true);
    resolveFn(structuredClone(DEPLOYMENT_NO_DESIGN));
    await flushPromises();
  });

  it('renders DesignGenerationPanel when DesignSetupPanel emits generate', async () => {
    seedSelectedProject();
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_NO_DESIGN));
    const w = mountView();
    await flushPromises();
    const setup = w.findComponent({ name: 'DesignSetupPanel' });
    setup.vm.$emit('generate', { artifact_ids: ['a1'], product_template_id: 't1' });
    await flushPromises();
    expect(w.findComponent({ name: 'DesignGenerationPanel' }).exists()).toBe(true);
    expect(w.findComponent({ name: 'DesignSetupPanel' }).exists()).toBe(false);
  });

  it('passes the generate body, deployment id, and deployment name to the generation panel', async () => {
    seedSelectedProject();
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_NO_DESIGN));
    const w = mountView();
    await flushPromises();
    const setup = w.findComponent({ name: 'DesignSetupPanel' });
    setup.vm.$emit('generate', { artifact_ids: ['a1'], product_template_id: 't1' });
    await flushPromises();
    const gen = w.findComponent({ name: 'DesignGenerationPanel' });
    expect(gen.props('body')).toEqual({ artifact_ids: ['a1'], product_template_id: 't1' });
    expect(gen.props('deploymentId')).toBe('d1');
    expect(gen.props('deploymentName')).toBe('MyProject');
  });

  it('refetches deployment and shows DesignPanel when DesignGenerationPanel emits done', async () => {
    seedSelectedProject();
    api.getProjectDeploymentSingle
      .mockResolvedValueOnce(structuredClone(DEPLOYMENT_NO_DESIGN))
      .mockResolvedValueOnce(structuredClone(DEPLOYMENT_WITH_DESIGN));
    const w = mountView();
    await flushPromises();
    const setup = w.findComponent({ name: 'DesignSetupPanel' });
    setup.vm.$emit('generate', { artifact_ids: [], product_template_id: null });
    await flushPromises();
    const gen = w.findComponent({ name: 'DesignGenerationPanel' });
    gen.vm.$emit('done');
    await flushPromises();
    expect(api.getProjectDeploymentSingle).toHaveBeenCalledTimes(2);
    expect(w.findComponent({ name: 'DesignPanel' }).exists()).toBe(true);
    expect(w.findComponent({ name: 'DesignGenerationPanel' }).exists()).toBe(false);
  });

  it('shows DesignSetupPanel again when generation finishes but no design_doc was created', async () => {
    seedSelectedProject();
    api.getProjectDeploymentSingle
      .mockResolvedValueOnce(structuredClone(DEPLOYMENT_NO_DESIGN))
      .mockResolvedValueOnce(structuredClone(DEPLOYMENT_NO_DESIGN));
    const w = mountView();
    await flushPromises();
    const setup = w.findComponent({ name: 'DesignSetupPanel' });
    setup.vm.$emit('generate', { artifact_ids: [], product_template_id: null });
    await flushPromises();
    const gen = w.findComponent({ name: 'DesignGenerationPanel' });
    gen.vm.$emit('done');
    await flushPromises();
    expect(w.findComponent({ name: 'DesignSetupPanel' }).exists()).toBe(true);
    expect(w.findComponent({ name: 'DesignGenerationPanel' }).exists()).toBe(false);
  });
});
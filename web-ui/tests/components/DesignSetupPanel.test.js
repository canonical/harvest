import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createRouter, createMemoryHistory } from 'vue-router';

const ARTIFACTS = [
  { id: 'a1', title: 'Requirements notes', kind: 'markdown', created_by: 'alice', created_at: new Date().toISOString() },
  { id: 'a2', title: 'Network diagram',    kind: 'pdf',      created_by: 'alice', created_at: new Date().toISOString() },
  { id: 'a3', title: 'Existing infra',     kind: 'terraform', created_by: 'assistant', created_at: new Date().toISOString() },
];

const TEMPLATES = [
  { id: 't1', name: 'Acme Gateway v3', description: 'standard rollout' },
  { id: 't2', name: 'Edge Cache',       description: 'cdn baseline' },
];

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    listProjectArtifacts:  vi.fn(),
    listTemplates:         vi.fn(),
    createProjectArtifact: vi.fn(),
  };
});

import DesignSetupPanel from '../../src/components/deployment/DesignSetupPanel.vue';
import * as api from '../../src/lib/api.js';

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/artifacts', component: { template: '<div />' } },
      { path: '/design',    component: { template: '<div />' } },
    ],
  });
}

async function mountPanel({ groupId = 'g1', deploymentId = 'd1', projectId = 'proj-1' } = {}) {
  const router = makeRouter();
  router.push('/design');
  await router.isReady();
  const w = mount(DesignSetupPanel, {
    props: { projectId, deploymentId, groupId },
    global: { plugins: [createPinia(), router] },
  });
  await flushPromises();
  return { w, router };
}

describe('DesignSetupPanel', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    api.listProjectArtifacts.mockResolvedValue(ARTIFACTS);
    api.listTemplates.mockResolvedValue(TEMPLATES);
  });

  it('loads project artifacts and group templates on mount', async () => {
    await mountPanel();
    expect(api.listProjectArtifacts).toHaveBeenCalledWith('proj-1');
    expect(api.listTemplates).toHaveBeenCalled();
  });

  it('renders as a full-page setup surface', async () => {
    const { w } = await mountPanel();
    expect(w.find('[data-testid="design-setup"]').classes()).toContain('design-setup');
  });

  it('renders numbered step labels for template and artifacts', async () => {
    const { w } = await mountPanel();
    expect(w.find('[data-testid="step-template"]').exists()).toBe(true);
    expect(w.find('[data-testid="step-artifacts"]').exists()).toBe(true);
    expect(w.find('[data-testid="step-template"]').text()).toMatch(/1/);
    expect(w.find('[data-testid="step-artifacts"]').text()).toMatch(/2/);
  });

  it('renders a product template selector populated with group templates, with none selected by default', async () => {
    const { w } = await mountPanel();
    const list = w.find('[data-testid="template-list"]');
    expect(list.exists()).toBe(true);
    expect(w.find('[data-testid="template-radio-t1"]').exists()).toBe(true);
    expect(w.find('[data-testid="template-radio-t2"]').exists()).toBe(true);
    expect(list.text()).toContain('Acme Gateway v3');
    expect(list.text()).toContain('Edge Cache');
    expect(w.find('[data-testid="template-radio-t1"]').element.checked).toBe(false);
    expect(w.find('[data-testid="template-radio-t2"]').element.checked).toBe(false);
  });

  it('selects a single product template at a time via radio buttons', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="template-radio-t1"]').setValue(true);
    expect(w.find('[data-testid="template-radio-t1"]').element.checked).toBe(true);
    await w.find('[data-testid="template-radio-t2"]').setValue(true);
    expect(w.find('[data-testid="template-radio-t1"]').element.checked).toBe(false);
    expect(w.find('[data-testid="template-radio-t2"]').element.checked).toBe(true);
  });

  it('renders the project artifacts as selectable rows', async () => {
    const { w } = await mountPanel();
    expect(w.find('[data-testid="artifact-checkbox-a1"]').exists()).toBe(true);
    expect(w.find('[data-testid="artifact-checkbox-a2"]').exists()).toBe(true);
    expect(w.find('[data-testid="artifact-checkbox-a3"]').exists()).toBe(true);
  });

  it('toggles artifact selection via checkboxes', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="artifact-checkbox-a1"]').setValue(true);
    await w.find('[data-testid="artifact-checkbox-a2"]').setValue(true);
    await w.find('[data-testid="artifact-checkbox-a1"]').setValue(false);
    expect(w.find('[data-testid="artifact-checkbox-a1"]').element.checked).toBe(false);
    expect(w.find('[data-testid="artifact-checkbox-a2"]').element.checked).toBe(true);
  });

  it('shows a selection count that updates as artifacts are toggled', async () => {
    const { w } = await mountPanel();
    expect(w.find('[data-testid="selection-count"]').text()).toMatch(/0 selected/i);
    await w.find('[data-testid="artifact-checkbox-a1"]').setValue(true);
    expect(w.find('[data-testid="selection-count"]').text()).toMatch(/1 selected/i);
    await w.find('[data-testid="artifact-checkbox-a3"]').setValue(true);
    expect(w.find('[data-testid="selection-count"]').text()).toMatch(/2 selected/i);
  });

  it('shows select-all and clear controls when artifacts exist', async () => {
    const { w } = await mountPanel();
    expect(w.find('[data-testid="select-all-artifacts"]').exists()).toBe(true);
    expect(w.find('[data-testid="clear-artifacts"]').exists()).toBe(true);
  });

  it('select-all selects every artifact and clear empties selection', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="select-all-artifacts"]').trigger('click');
    expect(w.find('[data-testid="selection-count"]').text()).toMatch(/3 selected/i);
    await w.find('[data-testid="clear-artifacts"]').trigger('click');
    expect(w.find('[data-testid="selection-count"]').text()).toMatch(/0 selected/i);
  });

  it('shows Create and Add buttons for managing artifacts without leaving the page', async () => {
    const { w } = await mountPanel();
    expect(w.find('[data-testid="create-artifact-btn"]').exists()).toBe(true);
    expect(w.find('[data-testid="add-artifact-btn"]').exists()).toBe(true);
  });

  it('opens the create-artifact modal, submits it, and selects the new artifact', async () => {
    const created = { id: 'new-1', title: 'New doc', kind: 'markdown', created_by: 'alice', created_at: new Date().toISOString() };
    api.createProjectArtifact.mockResolvedValue(created);
    api.listProjectArtifacts.mockResolvedValueOnce(ARTIFACTS).mockResolvedValue([...ARTIFACTS, created]);
    const { w } = await mountPanel();
    await w.find('[data-testid="create-artifact-btn"]').trigger('click');
    expect(w.find('[data-testid="create-artifact-modal"]').exists()).toBe(true);
    await w.find('[data-testid="create-artifact-title"]').setValue('New doc');
    await w.find('[data-testid="create-artifact-content"]').setValue('Some content');
    await w.find('[data-testid="submit-create-artifact"]').trigger('click');
    await flushPromises();
    expect(api.createProjectArtifact).toHaveBeenCalledWith('proj-1', {
      title: 'New doc',
      kind: 'markdown',
      content: 'Some content',
    });
    expect(w.find('[data-testid="create-artifact-modal"]').exists()).toBe(false);
    expect(w.find('[data-testid="artifact-checkbox-new-1"]').element.checked).toBe(true);
  });

  it('opens the add-artifact (upload) modal from the toolbar', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="add-artifact-btn"]').trigger('click');
    expect(w.find('[data-testid="upload-artifact-modal"]').exists()).toBe(true);
  });

  it('shows an inviting empty state with an Add action when there are no artifacts', async () => {
    api.listProjectArtifacts.mockResolvedValue([]);
    const { w } = await mountPanel();
    expect(w.find('[data-testid="artifacts-empty"]').exists()).toBe(true);
    const emptyAddBtn = w.find('[data-testid="add-artifact-btn-empty"]');
    expect(emptyAddBtn.exists()).toBe(true);
    expect(emptyAddBtn.classes()).toContain('p-button--positive');
    await emptyAddBtn.trigger('click');
    expect(w.find('[data-testid="upload-artifact-modal"]').exists()).toBe(true);
  });

  it('shows a generation summary line reflecting the current selection', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="template-radio-t1"]').setValue(true);
    await w.find('[data-testid="artifact-checkbox-a1"]').setValue(true);
    await w.find('[data-testid="artifact-checkbox-a2"]').setValue(true);
    const summary = w.find('[data-testid="generation-summary"]');
    expect(summary.exists()).toBe(true);
    expect(summary.text()).toContain('Acme Gateway v3');
    expect(summary.text()).toMatch(/2 artifacts/i);
  });

  it('prompts to select a template and an artifact when nothing is selected yet', async () => {
    const { w } = await mountPanel();
    const summary = w.find('[data-testid="generation-summary"]');
    expect(summary.text()).toMatch(/select a product template and at least one context artifact/i);
  });

  it('prompts to select an artifact when only a template is selected', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="template-radio-t1"]').setValue(true);
    const summary = w.find('[data-testid="generation-summary"]');
    expect(summary.text()).toMatch(/select a product template and at least one context artifact/i);
  });

  it('prompts to select a template when only an artifact is selected', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="artifact-checkbox-a1"]').setValue(true);
    const summary = w.find('[data-testid="generation-summary"]');
    expect(summary.text()).toMatch(/select a product template and at least one context artifact/i);
  });

  it('disables generate when neither a template nor an artifact is selected', async () => {
    const { w } = await mountPanel();
    expect(w.find('[data-testid="generate-design-btn"]').attributes('disabled')).toBeDefined();
  });

  it('disables generate when only a template is selected, with no artifacts', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="template-radio-t1"]').setValue(true);
    expect(w.find('[data-testid="generate-design-btn"]').attributes('disabled')).toBeDefined();
  });

  it('disables generate when only an artifact is selected, with no template', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="artifact-checkbox-a1"]').setValue(true);
    expect(w.find('[data-testid="generate-design-btn"]').attributes('disabled')).toBeDefined();
  });

  it('enables generate once both a template and an artifact are selected', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="template-radio-t1"]').setValue(true);
    expect(w.find('[data-testid="generate-design-btn"]').attributes('disabled')).toBeDefined();
    await w.find('[data-testid="artifact-checkbox-a1"]').setValue(true);
    expect(w.find('[data-testid="generate-design-btn"]').attributes('disabled')).toBeUndefined();
  });

  it('generate emits the selected artifact ids and template id', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="template-radio-t1"]').setValue(true);
    await w.find('[data-testid="artifact-checkbox-a1"]').setValue(true);
    await w.find('[data-testid="artifact-checkbox-a3"]').setValue(true);
    await w.find('[data-testid="generate-design-btn"]').trigger('click');
    await flushPromises();
    expect(w.emitted('generate')).toBeTruthy();
    expect(w.emitted('generate')[0][0]).toEqual({
      artifact_ids: ['a1', 'a3'],
      product_template_id: 't1',
    });
  });

  it('does not emit generate while only an artifact is selected and no template', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="artifact-checkbox-a2"]').setValue(true);
    await w.find('[data-testid="generate-design-btn"]').trigger('click');
    await flushPromises();
    expect(w.emitted('generate')).toBeFalsy();
  });
});

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
    listProjectArtifacts: vi.fn(),
    listTemplates:        vi.fn(),
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

  it('renders a product template selector populated with group templates', async () => {
    const { w } = await mountPanel();
    const select = w.find('[data-testid="template-select"]');
    expect(select.exists()).toBe(true);
    const options = select.findAll('option');
    expect(options.length).toBe(TEMPLATES.length + 1);
    expect(select.text()).toContain('Acme Gateway v3');
    expect(select.text()).toContain('Edge Cache');
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

  it('shows a link to the artifacts page for adding new artifacts', async () => {
    const { w } = await mountPanel();
    const links = w.findAll('[data-testid="artifacts-link"]');
    expect(links.length).toBeGreaterThan(0);
    expect(links[0].attributes('href')).toBe('/artifacts');
  });

  it('shows an inviting empty state with a link when there are no artifacts', async () => {
    api.listProjectArtifacts.mockResolvedValue([]);
    const { w } = await mountPanel();
    expect(w.find('[data-testid="artifacts-empty"]').exists()).toBe(true);
    expect(w.find('[data-testid="artifacts-link"]').exists()).toBe(true);
  });

  it('shows a generation summary line reflecting the current selection', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="template-select"]').setValue('t1');
    await w.find('[data-testid="artifact-checkbox-a1"]').setValue(true);
    await w.find('[data-testid="artifact-checkbox-a2"]').setValue(true);
    const summary = w.find('[data-testid="generation-summary"]');
    expect(summary.exists()).toBe(true);
    expect(summary.text()).toContain('Acme Gateway v3');
    expect(summary.text()).toMatch(/2 artifacts/i);
  });

  it('generate emits the selected artifact ids and template id', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="template-select"]').setValue('t1');
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

  it('generate works without a template (sends null)', async () => {
    const { w } = await mountPanel();
    await w.find('[data-testid="artifact-checkbox-a2"]').setValue(true);
    await w.find('[data-testid="generate-design-btn"]').trigger('click');
    await flushPromises();
    expect(w.emitted('generate')[0][0]).toEqual({
      artifact_ids: ['a2'],
      product_template_id: null,
    });
  });
});

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createRouter, createMemoryHistory } from 'vue-router';

const TEMPLATES = [
  { id: 't1', name: 'Gateway v3', description: 'standard rollout', created_by: 'alice', created_at: new Date().toISOString(), updated_at: new Date().toISOString() },
  { id: 't2', name: 'Edge Cache',  description: 'cdn baseline',      created_by: 'bob',   created_at: new Date().toISOString(), updated_at: new Date().toISOString() },
];

const TEMPLATE_DETAIL = {
  id: 't1', name: 'Gateway v3', description: 'standard rollout',
  content: JSON.stringify({
    design_template: '# 1. Introduction\n${CUSTOMER}',
    skills: [
      { name: 'juju', description: 'Deploy with Juju', content: '# Juju\nJuju is an operator framework.' },
    ],
    artifacts: [
      { name: 'main', kind: 'terraform', content: '{"main.tf":"resource null_resource x {}"}' },
    ],
  }),
  created_by: 'alice', created_at: new Date().toISOString(), updated_at: new Date().toISOString(),
};

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    listTemplates:   vi.fn(),
    getTemplate:     vi.fn(),
    deleteTemplate:  vi.fn(() => Promise.resolve()),
    uploadTemplate:  vi.fn(() => Promise.resolve({ id: 't3', name: 'New' })),
  };
});

import ProductTemplatesView from '../../src/views/ProductTemplatesView.vue';
import * as api from '../../src/lib/api.js';

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/product-templates', component: ProductTemplatesView },
    ],
  });
}

async function mountView() {
  const router = makeRouter();
  router.push('/product-templates');
  await router.isReady();
  const w = mount(ProductTemplatesView, { global: { plugins: [createPinia(), router] } });
  await flushPromises();
  return w;
}

describe('ProductTemplatesView', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    api.listTemplates.mockResolvedValue(TEMPLATES);
    api.getTemplate.mockResolvedValue(TEMPLATE_DETAIL);
    api.deleteTemplate.mockResolvedValue();
    api.uploadTemplate.mockResolvedValue({ id: 't3', name: 'New' });
  });

  it('renders the page header and upload button', async () => {
    const w = await mountView();
    expect(w.text()).toMatch(/product templates/i);
    expect(w.find('[data-testid="upload-template-btn"]').exists()).toBe(true);
  });

  it('lists templates after load', async () => {
    const w = await mountView();
    expect(w.text()).toContain('Gateway v3');
    expect(w.text()).toContain('Edge Cache');
  });

  it('shows empty state when no templates exist', async () => {
    api.listTemplates.mockResolvedValue([]);
    const w = await mountView();
    expect(w.text()).toMatch(/no templates/i);
  });

  it('selecting a template loads its detail with skills and artifacts', async () => {
    const w = await mountView();
    const items = w.findAll('[data-testid^="template-item-"]');
    await items[0].trigger('click');
    await flushPromises();
    expect(api.getTemplate).toHaveBeenCalledWith('t1');
    expect(w.text()).toContain('juju');
    expect(w.text()).toContain('Deploy with Juju');
    expect(w.text()).toContain('main');
    expect(w.text()).toContain('Terraform');
    expect(w.text()).toContain('Design template');
  });

  it('opens the upload modal with a dropzone', async () => {
    const w = await mountView();
    await w.find('[data-testid="upload-template-btn"]').trigger('click');
    expect(w.find('[data-testid="upload-template-modal"]').exists()).toBe(true);
    expect(w.find('[data-testid="template-dropzone"]').exists()).toBe(true);
  });

  it('uploads a .harvest file and refreshes the list', async () => {
    const w = await mountView();
    await w.find('[data-testid="upload-template-btn"]').trigger('click');
    const file = new File(['fake-zip'], 'test.harvest', { type: 'application/octet-stream' });
    Object.defineProperty(w.find('[data-testid="template-file-input"]').element, 'files', { value: [file], configurable: true, writable: true });
    await w.find('[data-testid="template-file-input"]').trigger('change');
    await flushPromises();
    await w.find('[data-testid="submit-upload-template"]').trigger('click');
    await flushPromises();
    await flushPromises();
    expect(api.uploadTemplate).toHaveBeenCalledWith(file);
  });

  it('rejects non-.harvest files in the upload modal', async () => {
    const w = await mountView();
    await w.find('[data-testid="upload-template-btn"]').trigger('click');
    const file = new File(['x'], 'image.png', { type: 'image/png' });
    Object.defineProperty(w.find('[data-testid="template-file-input"]').element, 'files', { value: [file], configurable: true, writable: true });
    await w.find('[data-testid="template-file-input"]').trigger('change');
    await flushPromises();
    expect(w.find('[data-testid="upload-template-modal"]').text()).toMatch(/\.harvest/i);
  });

  it('deletes a template after confirmation', async () => {
    const w = await mountView();
    const items = w.findAll('[data-testid^="template-item-"]');
    await items[0].trigger('click');
    await flushPromises();
    await w.find('[data-testid="delete-template-btn"]').trigger('click');
    await w.find('[data-testid="confirm-delete-template"]').trigger('click');
    await flushPromises();
    expect(api.deleteTemplate).toHaveBeenCalledWith('t1');
  });

  it('shows an error when upload fails', async () => {
    api.uploadTemplate.mockRejectedValueOnce(new Error('invalid archive'));
    const w = await mountView();
    await w.find('[data-testid="upload-template-btn"]').trigger('click');
    const file = new File(['bad'], 'bad.harvest', { type: 'application/octet-stream' });
    Object.defineProperty(w.find('[data-testid="template-file-input"]').element, 'files', { value: [file], configurable: true, writable: true });
    await w.find('[data-testid="template-file-input"]').trigger('change');
    await flushPromises();
    await w.find('[data-testid="submit-upload-template"]').trigger('click');
    await flushPromises();
    await flushPromises();
    expect(w.find('[data-testid="upload-template-modal"]').text()).toContain('invalid archive');
  });
});
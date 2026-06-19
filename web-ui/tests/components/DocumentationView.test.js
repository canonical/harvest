import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import DocumentationView from '../../src/views/DocumentationView.vue';

const REPOS = [
  { name: 'myrepo', versions: ['v1.0', 'v2.0'], url: 'https://github.com/org/myrepo' },
];

const DOC_INDEX = {
  repo: 'myrepo', version: 'v1.0',
  sections: {
    tutorials:     [{ filename: 'getting-started.md', title: 'Getting Started' }],
    'how-to-guides': [],
    explanations:  [],
    reference:     [],
  },
};

function mockFetch(extraResponses = []) {
  let call = 0;
  const responses = [{ body: REPOS }, ...extraResponses];
  global.fetch = vi.fn(() => {
    const r = responses[call++] ?? responses.at(-1);
    return Promise.resolve({
      ok: r.ok ?? true,
      status: r.status ?? 200,
      json: () => Promise.resolve(r.body ?? {}),
      text: () => Promise.resolve(r.text ?? ''),
    });
  });
}

describe('DocumentationView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('renders repo select', async () => {
    mockFetch();
    const w = mount(DocumentationView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    expect(w.find('select').exists()).toBe(true);
  });

  it('populates repo options from API', async () => {
    mockFetch();
    const w = mount(DocumentationView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    const options = w.findAll('option');
    expect(options.some(o => o.text().includes('myrepo'))).toBe(true);
  });

  it('shows version select disabled before repo chosen', async () => {
    mockFetch();
    const w = mount(DocumentationView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    const selects = w.findAll('select');
    expect(selects.length).toBeGreaterThanOrEqual(2);
    expect(selects[1].attributes('disabled')).toBeDefined();
  });

  it('loads doc index on version select', async () => {
    mockFetch([{ body: DOC_INDEX }]);
    const w = mount(DocumentationView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    // pick repo
    await w.findAll('select')[0].setValue('myrepo');
    await w.findAll('select')[0].trigger('change');
    // pick version
    await w.findAll('select')[1].setValue('v1.0');
    await w.findAll('select')[1].trigger('change');
    await flushPromises();
    expect(w.text()).toContain('Getting Started');
  });

  it('shows welcome state after index loaded but no page selected', async () => {
    mockFetch([{ body: DOC_INDEX }]);
    const w = mount(DocumentationView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    await w.findAll('select')[0].setValue('myrepo');
    await w.findAll('select')[0].trigger('change');
    await w.findAll('select')[1].setValue('v1.0');
    await w.findAll('select')[1].trigger('change');
    await flushPromises();
    expect(w.find('.doc-welcome, .doc-content-area').exists() || w.text().includes('page')).toBe(true);
  });
});

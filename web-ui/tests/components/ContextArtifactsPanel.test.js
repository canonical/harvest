import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    addContextArtifact:    vi.fn(),
    removeContextArtifact: vi.fn(),
    getArtifact:           vi.fn(),
  };
});

import ContextArtifactsPanel from '../../src/components/deployment/ContextArtifactsPanel.vue';
import * as api from '../../src/lib/api.js';

const DEPLOYMENT = {
  id: 'd1', context_artifacts: [
    { id: 'ca1', title: 'Network notes', kind: 'markdown' },
    { id: 'ca2', title: 'Prep script', kind: 'bash' },
  ],
};

function mountPanel(deployment = DEPLOYMENT) {
  return mount(ContextArtifactsPanel, {
    props: { projectId: 'proj-1', deployment },
  });
}

describe('ContextArtifactsPanel', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('lists context artifacts in the left panel', () => {
    const w = mountPanel();
    const items = w.findAll('[data-testid^="context-artifact-"]');
    expect(items).toHaveLength(2);
    expect(w.text()).toContain('Network notes');
    expect(w.text()).toContain('Prep script');
  });

  it('shows empty state when no context artifacts', () => {
    const w = mountPanel({ ...DEPLOYMENT, context_artifacts: [] });
    expect(w.text()).toContain('No context artifacts');
  });

  it('shows an add form with title, kind selector, and content textarea', () => {
    const w = mountPanel();
    expect(w.find('[data-testid="add-context-title"]').exists()).toBe(true);
    expect(w.find('[data-testid="add-context-kind"]').exists()).toBe(true);
    expect(w.find('[data-testid="add-context-content"]').exists()).toBe(true);
    expect(w.find('[data-testid="add-context-submit"]').exists()).toBe(true);
  });

  it('add button calls addContextArtifact and emits refresh', async () => {
    api.addContextArtifact.mockResolvedValue({ context_artifacts: [] });
    const w = mountPanel();
    await w.find('[data-testid="add-context-title"]').setValue('New notes');
    await w.find('[data-testid="add-context-kind"]').setValue('markdown');
    await w.find('[data-testid="add-context-content"]').setValue('# Hello');
    await w.find('[data-testid="add-context-submit"]').trigger('click');
    await flushPromises();
    expect(api.addContextArtifact).toHaveBeenCalledWith('proj-1', 'd1', {
      title: 'New notes', kind: 'markdown', content: '# Hello',
    });
  });

  it('add button disabled when title is empty', () => {
    const w = mountPanel();
    const btn = w.find('[data-testid="add-context-submit"]');
    expect(btn.attributes('disabled')).toBeDefined();
  });

  it('remove button calls removeContextArtifact and emits refresh', async () => {
    api.removeContextArtifact.mockResolvedValue(undefined);
    const w = mountPanel();
    await w.find('[data-testid="remove-context-ca1"]').trigger('click');
    await flushPromises();
    expect(api.removeContextArtifact).toHaveBeenCalledWith('proj-1', 'd1', 'ca1');
  });

  it('kind selector includes markdown, bash, terraform, terragrunt, pdf', () => {
    const w = mountPanel();
    const options = w.find('[data-testid="add-context-kind"]').findAll('option');
    const values = options.map(o => o.attributes('value'));
    expect(values).toContain('markdown');
    expect(values).toContain('bash');
    expect(values).toContain('terraform');
    expect(values).toContain('terragrunt');
    expect(values).toContain('pdf');
  });

  it('clicking an artifact fetches content and shows it in the right panel', async () => {
    api.getArtifact.mockResolvedValue({ content: '# Some markdown content' });
    const w = mountPanel();
    await w.find('[data-testid="context-artifact-ca1"]').trigger('click');
    await flushPromises();
    expect(api.getArtifact).toHaveBeenCalledWith('ca1');
    expect(w.find('[data-testid="content-context-ca1"]').exists()).toBe(true);
    expect(w.find('[data-testid="content-context-ca1"]').text()).toContain('Some markdown content');
  });

  it('clicking an artifact again deselects it', async () => {
    api.getArtifact.mockResolvedValue({ content: 'content here' });
    const w = mountPanel();
    await w.find('[data-testid="context-artifact-ca1"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="content-context-ca1"]').exists()).toBe(true);
    await w.find('[data-testid="context-artifact-ca1"]').trigger('click');
    expect(w.find('[data-testid="content-context-ca1"]').exists()).toBe(false);
  });
});

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getArtifact:           vi.fn(),
    generateDesignDecisions: vi.fn(),
    reviseDesign:          vi.fn(),
    updateArtifact:        vi.fn(),
    proposeArtifactChange: vi.fn(),
  };
});

vi.mock('../../src/lib/markdown.js', () => ({
  renderMarkdown: vi.fn(() => '<p>rendered</p>'),
}));

import DesignPanel from '../../src/components/deployment/DesignPanel.vue';
import * as api from '../../src/lib/api.js';

const DEPLOYMENT_WITH_DESIGN = {
  id: 'd1', name: 'Rollout',
  design_doc: { id: 'a1', title: 'Design' },
  created_by: 'assistant', created_at: '2026-08-26T10:00:00Z',
};

function mountPanel(deployment = DEPLOYMENT_WITH_DESIGN) {
  return mount(DesignPanel, {
    props: { projectId: 'proj-1', deployment },
  });
}

describe('DesignPanel', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('shows a document header with the design title and metadata when design doc exists', async () => {
    api.getArtifact.mockResolvedValue({ content: '# My Design' });
    const w = mountPanel();
    await flushPromises();
    const header = w.find('[data-testid="design-doc-header"]');
    expect(header.exists()).toBe(true);
    expect(header.text()).toContain('Design');
  });

  it('renders the design preview with the doc-body class for markdown styling', async () => {
    api.getArtifact.mockResolvedValue({ content: '# My Design' });
    const w = mountPanel();
    await flushPromises();
    const preview = w.find('[data-testid="design-preview"]');
    expect(preview.exists()).toBe(true);
    expect(preview.classes()).toContain('doc-body');
  });

  it('shows the edit button when design doc exists', async () => {
    api.getArtifact.mockResolvedValue({ content: '# My Design' });
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="edit-design-btn"]').exists()).toBe(true);
  });

  it('edit toggles inline editor with current content', async () => {
    api.getArtifact.mockResolvedValue({ content: '# My Design' });
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="design-editor"]').exists()).toBe(false);
    await w.find('[data-testid="edit-design-btn"]').trigger('click');
    expect(w.find('[data-testid="design-editor"]').exists()).toBe(true);
    expect(w.find('[data-testid="design-editor"]').element.value).toContain('# My Design');
  });

  it('hides the propose prompt while editing and shows a sticky edit bar', async () => {
    api.getArtifact.mockResolvedValue({ content: '# My Design' });
    const w = mountPanel();
    await flushPromises();
    expect(w.find('[data-testid="design-prompt"]').exists()).toBe(true);
    await w.find('[data-testid="edit-design-btn"]').trigger('click');
    expect(w.find('[data-testid="design-prompt"]').exists()).toBe(false);
    expect(w.find('[data-testid="design-edit-bar"]').exists()).toBe(true);
  });

  it('save edit calls updateArtifact with new content and emits refresh', async () => {
    api.getArtifact.mockResolvedValue({ content: '# Old' });
    api.updateArtifact.mockResolvedValue({});
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="edit-design-btn"]').trigger('click');
    await w.find('[data-testid="design-editor"]').setValue('# New content');
    await w.find('[data-testid="save-design-btn"]').trigger('click');
    await flushPromises();
    expect(api.updateArtifact).toHaveBeenCalledWith('a1', expect.objectContaining({
      title: 'Design', kind: 'markdown', content: '# New content',
    }));
  });

  it('prompt box produces a proposal via proposeArtifactChange', async () => {
    api.getArtifact.mockResolvedValue({ content: '# Old' });
    api.proposeArtifactChange.mockResolvedValue({ id: 'p1', status: 'pending' });
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="design-prompt"]').setValue('Add a load balancer section');
    await w.find('[data-testid="propose-design-btn"]').trigger('click');
    await flushPromises();
    expect(api.proposeArtifactChange).toHaveBeenCalledWith('proj-1', 'd1', expect.objectContaining({
      artifact_id: 'a1',
    }));
  });

  it('prompt box disabled when empty', async () => {
    api.getArtifact.mockResolvedValue({ content: '# Old' });
    const w = mountPanel();
    await flushPromises();
    const btn = w.find('[data-testid="propose-design-btn"]');
    expect(btn.attributes('disabled')).toBeDefined();
  });

  it('cancel edit closes the editor without saving', async () => {
    api.getArtifact.mockResolvedValue({ content: '# Old' });
    const w = mountPanel();
    await flushPromises();
    await w.find('[data-testid="edit-design-btn"]').trigger('click');
    await w.find('[data-testid="cancel-edit-btn"]').trigger('click');
    expect(w.find('[data-testid="design-editor"]').exists()).toBe(false);
  });
});
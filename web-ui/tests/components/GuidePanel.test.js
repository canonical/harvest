import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getArtifact:    vi.fn(),
    updateArtifact: vi.fn(),
  };
});

vi.mock('../../src/lib/markdown.js', () => ({
  renderMarkdown: vi.fn(() => '<p>rendered</p>'),
}));

import GuidePanel from '../../src/components/deployment/GuidePanel.vue';
import * as api from '../../src/lib/api.js';

const DEPLOYMENT_NO_GUIDE = { id: 'd1', guide: null };
const DEPLOYMENT_WITH_GUIDE = { id: 'd1', guide: { id: 'g1', title: 'Deployment Guide' } };

function mountPanel(deployment = DEPLOYMENT_NO_GUIDE) {
  return mount(GuidePanel, { props: { projectId: 'proj-1', deployment } });
}

describe('GuidePanel', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('shows empty state when no guide exists', () => {
    const w = mountPanel();
    expect(w.text()).toContain('No guide');
  });

  it('shows rendered guide and edit button when guide exists', async () => {
    api.getArtifact.mockResolvedValue({ content: '# Guide' });
    const w = mountPanel(DEPLOYMENT_WITH_GUIDE);
    await flushPromises();
    expect(w.find('[data-testid="guide-preview"]').exists()).toBe(true);
    expect(w.find('[data-testid="edit-guide-btn"]').exists()).toBe(true);
  });

  it('edit toggles inline editor', async () => {
    api.getArtifact.mockResolvedValue({ content: '# Guide' });
    const w = mountPanel(DEPLOYMENT_WITH_GUIDE);
    await flushPromises();
    expect(w.find('[data-testid="guide-editor"]').exists()).toBe(false);
    await w.find('[data-testid="edit-guide-btn"]').trigger('click');
    expect(w.find('[data-testid="guide-editor"]').exists()).toBe(true);
  });

  it('save edit calls updateArtifact with new content', async () => {
    api.getArtifact.mockResolvedValue({ content: '# Old' });
    api.updateArtifact.mockResolvedValue({});
    const w = mountPanel(DEPLOYMENT_WITH_GUIDE);
    await flushPromises();
    await w.find('[data-testid="edit-guide-btn"]').trigger('click');
    await w.find('[data-testid="guide-editor"]').setValue('# New guide');
    await w.find('[data-testid="save-guide-btn"]').trigger('click');
    await flushPromises();
    expect(api.updateArtifact).toHaveBeenCalledWith('g1', expect.objectContaining({
      content: '# New guide',
    }));
  });
});

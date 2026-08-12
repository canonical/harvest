import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { nextTick } from 'vue';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getArtifact:               vi.fn(),
    generateDesign:            vi.fn(),
    generateDesignDecisions:   vi.fn(),
    reviseDesign:              vi.fn(),
  };
});

import DesignPhase from '../../src/components/deployment/DesignPhase.vue';
import * as api from '../../src/lib/api.js';

const NO_DESIGN = { id: 'd1', design_doc: null };
const WITH_DESIGN = { id: 'd1', design_doc: { id: 'a1', title: 'Design' } };

function mountPhase(deployment) {
  return mount(DesignPhase, { props: { projectId: 'proj-1', deployment } });
}

describe('DesignPhase', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('shows a Generate design button when there is no design doc yet', () => {
    const w = mountPhase(NO_DESIGN);
    expect(w.find('[data-testid="generate-design-btn"]').exists()).toBe(true);
  });

  it('clicking Generate design calls generateDesign and emits refresh', async () => {
    api.generateDesign.mockResolvedValue({});
    const w = mountPhase(NO_DESIGN);
    await w.find('[data-testid="generate-design-btn"]').trigger('click');
    await flushPromises();

    expect(api.generateDesign).toHaveBeenCalledWith('proj-1', 'd1');
    expect(w.emitted('refresh')).toBeTruthy();
  });

  it('shows an error when design generation fails', async () => {
    api.generateDesign.mockRejectedValue(new Error('llm unavailable'));
    const w = mountPhase(NO_DESIGN);
    await w.find('[data-testid="generate-design-btn"]').trigger('click');
    await flushPromises();
    expect(w.text()).toContain('llm unavailable');
  });

  it('renders the design document content on the right when a design doc exists', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '# Design\n\nUse one VM.' });
    const w = mountPhase(WITH_DESIGN);
    await flushPromises();

    expect(api.getArtifact).toHaveBeenCalledWith('a1');
    expect(w.find('.design-phase__right').html()).toContain('Use one VM.');
  });

  it('Get design decisions renders one input per decision, seeded with the suggested answer', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '# Design' });
    api.generateDesignDecisions.mockResolvedValue({
      decisions: [{ id: 'sizing', text: 'Confirm VM size', suggested: 'medium' }],
    });
    const w = mountPhase(WITH_DESIGN);
    await flushPromises();

    await w.find('[data-testid="get-decisions-btn"]').trigger('click');
    await flushPromises();

    expect(w.text()).toContain('Confirm VM size');
    expect(w.find('#design-d-sizing').element.value).toBe('medium');
  });

  it('Revise design sends answered decisions and instructions, then reloads content', async () => {
    api.getArtifact
      .mockResolvedValueOnce({ id: 'a1', content: '# Design v1' })
      .mockResolvedValueOnce({ id: 'a1', content: '# Design v2' });
    api.generateDesignDecisions.mockResolvedValue({
      decisions: [{ id: 'sizing', text: 'Confirm VM size', suggested: 'medium' }],
    });
    api.reviseDesign.mockResolvedValue({});
    const w = mountPhase(WITH_DESIGN);
    await flushPromises();

    await w.find('[data-testid="get-decisions-btn"]').trigger('click');
    await flushPromises();
    await w.find('#design-instructions').setValue('use spot instances');

    await w.find('[data-testid="revise-design-btn"]').trigger('click');
    await flushPromises();

    expect(api.reviseDesign).toHaveBeenCalledWith('proj-1', 'd1', {
      decisions: [{ question: 'Confirm VM size', answer: 'medium' }],
      instructions: 'use spot instances',
    });
    expect(w.find('.design-phase__right').html()).toContain('Design v2');
    expect(w.emitted('refresh')).toBeTruthy();
  });

  it('Revise design button is disabled until a decision or instruction is provided', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '# Design' });
    const w = mountPhase(WITH_DESIGN);
    await flushPromises();
    expect(w.find('[data-testid="revise-design-btn"]').attributes('disabled')).toBeDefined();

    await w.find('#design-instructions').setValue('note');
    expect(w.find('[data-testid="revise-design-btn"]').attributes('disabled')).toBeUndefined();
  });

  it('shows a busy indicator while generating the design, then clears it', async () => {
    let resolveGenerate;
    api.generateDesign.mockReturnValue(new Promise(r => { resolveGenerate = r; }));
    const w = mountPhase(NO_DESIGN);
    await w.find('[data-testid="generate-design-btn"]').trigger('click');
    await nextTick();

    expect(w.find('[data-testid="busy-status"]').exists()).toBe(true);
    expect(w.text()).toContain('Generating design…');

    resolveGenerate({});
    await flushPromises();
    expect(w.find('[data-testid="busy-status"]').exists()).toBe(false);
  });

  it('shows a busy indicator while getting design decisions', async () => {
    api.getArtifact.mockResolvedValue({ id: 'a1', content: '# Design' });
    let resolveDecisions;
    api.generateDesignDecisions.mockReturnValue(new Promise(r => { resolveDecisions = r; }));
    const w = mountPhase(WITH_DESIGN);
    await flushPromises();

    await w.find('[data-testid="get-decisions-btn"]').trigger('click');
    await nextTick();
    expect(w.text()).toContain('Getting design decisions…');

    resolveDecisions({ decisions: [] });
    await flushPromises();
    expect(w.find('[data-testid="busy-status"]').exists()).toBe(false);
  });
});

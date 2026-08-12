import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { nextTick } from 'vue';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    generateEnvironmentQuestions: vi.fn(),
    updateProjectDeployment:      vi.fn(),
  };
});

import EnvironmentPhase from '../../src/components/deployment/EnvironmentPhase.vue';
import * as api from '../../src/lib/api.js';

const DEPLOYMENT = { id: 'd1', environment_description: '' };

function mountPhase(deployment = DEPLOYMENT) {
  return mount(EnvironmentPhase, { props: { projectId: 'proj-1', deployment } });
}

describe('EnvironmentPhase', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('shows a Generate questions button initially', () => {
    const w = mountPhase();
    expect(w.text()).toContain('Generate questions');
  });

  it('generating questions renders one input per question', async () => {
    api.generateEnvironmentQuestions.mockResolvedValue({
      questions: [{ id: 'racks', text: 'How many racks?' }, { id: 'net', text: 'What network topology?' }],
    });
    const w = mountPhase();
    await w.find('[data-testid="generate-questions-btn"]').trigger('click');
    await flushPromises();

    expect(api.generateEnvironmentQuestions).toHaveBeenCalledWith('proj-1', 'd1');
    expect(w.text()).toContain('How many racks?');
    expect(w.text()).toContain('What network topology?');
    expect(w.find('#env-q-racks').exists()).toBe(true);
  });

  it('shows an error when question generation fails', async () => {
    api.generateEnvironmentQuestions.mockRejectedValue(new Error('boom'));
    const w = mountPhase();
    await w.find('[data-testid="generate-questions-btn"]').trigger('click');
    await flushPromises();
    expect(w.text()).toContain('boom');
  });

  it('saving composes answered questions and notes into environment_description', async () => {
    api.generateEnvironmentQuestions.mockResolvedValue({ questions: [{ id: 'racks', text: 'How many racks?' }] });
    api.updateProjectDeployment.mockResolvedValue({ ok: true });
    const w = mountPhase();
    await w.find('[data-testid="generate-questions-btn"]').trigger('click');
    await flushPromises();

    await w.find('#env-q-racks').setValue('3');
    await w.find('#environment-notes').setValue('air-gapped site');

    await w.find('[data-testid="save-environment-btn"]').trigger('click');
    await flushPromises();

    expect(api.updateProjectDeployment).toHaveBeenCalledWith('proj-1', 'd1', {
      environment_description: 'Q: How many racks?\nA: 3\n\nair-gapped site',
    });
    expect(w.emitted('refresh')).toBeTruthy();
  });

  it('saving without generating questions just saves the notes', async () => {
    api.updateProjectDeployment.mockResolvedValue({ ok: true });
    const w = mountPhase();
    await w.find('#environment-notes').setValue('just notes');
    await w.find('[data-testid="save-environment-btn"]').trigger('click');
    await flushPromises();

    expect(api.updateProjectDeployment).toHaveBeenCalledWith('proj-1', 'd1', { environment_description: 'just notes' });
  });

  it('seeds notes from the existing environment_description', () => {
    const w = mountPhase({ id: 'd1', environment_description: 'pre-existing notes' });
    expect(w.find('#environment-notes').element.value).toBe('pre-existing notes');
  });

  it('shows a busy indicator while generating questions, then clears it', async () => {
    let resolveGenerate;
    api.generateEnvironmentQuestions.mockReturnValue(new Promise(r => { resolveGenerate = r; }));
    const w = mountPhase();
    await w.find('[data-testid="generate-questions-btn"]').trigger('click');
    await nextTick();

    expect(w.find('[data-testid="busy-status"]').exists()).toBe(true);
    expect(w.text()).toContain('Generating environment questions…');

    resolveGenerate({ questions: [] });
    await flushPromises();
    expect(w.find('[data-testid="busy-status"]').exists()).toBe(false);
  });

  it('shows a busy indicator while saving', async () => {
    let resolveSave;
    api.updateProjectDeployment.mockReturnValue(new Promise(r => { resolveSave = r; }));
    const w = mountPhase();
    await w.find('[data-testid="save-environment-btn"]').trigger('click');
    await nextTick();

    expect(w.text()).toContain('Saving environment description…');

    resolveSave({ ok: true });
    await flushPromises();
    expect(w.find('[data-testid="busy-status"]').exists()).toBe(false);
  });
});

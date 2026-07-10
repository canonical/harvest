import { describe, it, expect, beforeEach } from 'vitest';
import { mount } from '@vue/test-utils';
import LlmModelPicker from '../../src/components/chat/LlmModelPicker.vue';
import { useLlmStore } from '../../src/stores/llm.js';

function setProviders(providers) {
  const llm = useLlmStore();
  llm.providers = providers;
  return llm;
}

const providers = [
  {
    id: 'anthropic-main', kind: 'anthropic', default_model: 'claude-sonnet-5',
    models: [
      { id: 'claude-sonnet-5', display_name: 'Claude Sonnet 5' },
      { id: 'claude-opus-5', display_name: 'Claude Opus 5' },
    ],
  },
  {
    id: 'gemini-1', kind: 'gemini', default_model: 'gemini-2.5-flash',
    models: [
      { id: 'gemini-2.5-flash', display_name: 'Gemini 2.5 Flash' },
      { id: 'gemini-2.5-pro', display_name: 'Gemini 2.5 Pro' },
    ],
  },
];

describe('LlmModelPicker', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('shows the highest-precedence provider model as the trigger label with no selection', () => {
    setProviders(providers);
    const w = mount(LlmModelPicker);
    expect(w.find('.llm-picker__trigger').text()).toContain('Claude Sonnet 5');
  });

  it('prefixes the trigger label with a + to signal it is clickable', () => {
    setProviders(providers);
    const w = mount(LlmModelPicker);
    expect(w.find('.llm-picker__trigger-icon').text()).toBe('+');
  });

  it('is closed by default', () => {
    setProviders(providers);
    const w = mount(LlmModelPicker);
    expect(w.find('.llm-picker__panel').exists()).toBe(false);
  });

  it('opens the panel on trigger click', async () => {
    setProviders(providers);
    const w = mount(LlmModelPicker);
    await w.find('.llm-picker__trigger').trigger('click');
    expect(w.find('.llm-picker__panel').exists()).toBe(true);
    expect(w.find('.llm-picker__search').exists()).toBe(true);
  });

  it('lists a flattened set of all provider/model entries, with no Auto option', async () => {
    setProviders(providers);
    const w = mount(LlmModelPicker);
    await w.find('.llm-picker__trigger').trigger('click');
    const options = w.findAll('.llm-picker__option');
    expect(options).toHaveLength(4);
    expect(w.find('.llm-picker__option--auto').exists()).toBe(false);
    expect(w.text()).not.toContain('Auto (default)');
    expect(w.text()).toContain('Claude Sonnet 5');
    expect(w.text()).toContain('Gemini 2.5 Flash');
  });

  it('shows the configured provider name instead of kind when present', async () => {
    setProviders([
      {
        id: 'oai-lemonade', kind: 'openai-compatible', name: 'Lemonade (local)', default_model: 'm',
        models: [{ id: 'm', display_name: 'Mistral 3B' }],
      },
    ]);
    const w = mount(LlmModelPicker);
    await w.find('.llm-picker__trigger').trigger('click');
    const option = w.find('.llm-picker__option--model');
    expect(option.text()).toContain('Lemonade (local)');
    expect(option.text()).not.toContain('openai-compatible');
  });

  it('falls back to kind for the option label when no name is configured', async () => {
    setProviders(providers);
    const w = mount(LlmModelPicker);
    await w.find('.llm-picker__trigger').trigger('click');
    const option = w.find('.llm-picker__option--model');
    expect(option.text()).toContain('anthropic');
  });

  it('matches provider name in the search filter', async () => {
    setProviders([
      {
        id: 'oai-lemonade', kind: 'openai-compatible', name: 'Lemonade (local)', default_model: 'm',
        models: [{ id: 'm', display_name: 'Mistral 3B' }],
      },
      ...providers,
    ]);
    const w = mount(LlmModelPicker);
    await w.find('.llm-picker__trigger').trigger('click');
    await w.find('.llm-picker__search').setValue('lemonade');
    const options = w.findAll('.llm-picker__option--model');
    expect(options).toHaveLength(1);
    expect(options[0].text()).toContain('Mistral 3B');
  });

  it('filters the list as the user types', async () => {
    setProviders(providers);
    const w = mount(LlmModelPicker);
    await w.find('.llm-picker__trigger').trigger('click');
    await w.find('.llm-picker__search').setValue('opus');
    const options = w.findAll('.llm-picker__option--model');
    expect(options).toHaveLength(1);
    expect(options[0].text()).toContain('Claude Opus 5');
  });

  it('shows an empty state when nothing matches the filter', async () => {
    setProviders(providers);
    const w = mount(LlmModelPicker);
    await w.find('.llm-picker__trigger').trigger('click');
    await w.find('.llm-picker__search').setValue('nonexistent-model-xyz');
    expect(w.find('.llm-picker__empty').exists()).toBe(true);
  });

  it('clicking a model option selects it, updates the store, and closes the panel', async () => {
    const llm = setProviders(providers);
    const w = mount(LlmModelPicker);
    await w.find('.llm-picker__trigger').trigger('click');
    await w.find('.llm-picker__search').setValue('opus');
    await w.find('.llm-picker__option--model').trigger('click');

    expect(llm.selection).toEqual({ providerId: 'anthropic-main', model: 'claude-opus-5' });
    expect(w.find('.llm-picker__panel').exists()).toBe(false);
    expect(w.find('.llm-picker__trigger').text()).toContain('Claude Opus 5');
  });

  it('Escape closes the panel', async () => {
    setProviders(providers);
    const w = mount(LlmModelPicker);
    await w.find('.llm-picker__trigger').trigger('click');
    await w.find('.llm-picker__search').trigger('keydown', { key: 'Escape' });
    expect(w.find('.llm-picker__panel').exists()).toBe(false);
  });

  it('ArrowDown then Enter selects the first filtered entry', async () => {
    const llm = setProviders(providers);
    const w = mount(LlmModelPicker);
    await w.find('.llm-picker__trigger').trigger('click');
    await w.find('.llm-picker__search').setValue('gemini');
    await w.find('.llm-picker__search').trigger('keydown', { key: 'ArrowDown' });
    await w.find('.llm-picker__search').trigger('keydown', { key: 'Enter' });

    expect(llm.selection).toEqual({ providerId: 'gemini-1', model: 'gemini-2.5-flash' });
  });

  it('Enter with no arrow presses does not change the selection', async () => {
    const llm = setProviders(providers);
    llm.setSelection('gemini-1', 'gemini-2.5-flash');
    const w = mount(LlmModelPicker);
    await w.find('.llm-picker__trigger').trigger('click');
    await w.find('.llm-picker__search').trigger('keydown', { key: 'Enter' });

    expect(llm.selection).toEqual({ providerId: 'gemini-1', model: 'gemini-2.5-flash' });
  });

  it('clicking outside the component closes the panel', async () => {
    setProviders(providers);
    const w = mount(LlmModelPicker, { attachTo: document.body });
    await w.find('.llm-picker__trigger').trigger('click');
    expect(w.find('.llm-picker__panel').exists()).toBe(true);

    document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    await w.vm.$nextTick();

    expect(w.find('.llm-picker__panel').exists()).toBe(false);
    w.unmount();
  });

  it('reflects a selection already present in the store on mount', () => {
    const llm = setProviders(providers);
    llm.setSelection('gemini-1', 'gemini-2.5-pro');
    const w = mount(LlmModelPicker);
    expect(w.find('.llm-picker__trigger').text()).toContain('Gemini 2.5 Pro');
  });

  it('renders nothing when there are no providers', () => {
    setProviders([]);
    const w = mount(LlmModelPicker);
    expect(w.find('.llm-picker').exists()).toBe(false);
  });
});

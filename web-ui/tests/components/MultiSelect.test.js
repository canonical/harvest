import { describe, it, expect, afterEach } from 'vitest';
import { mount } from '@vue/test-utils';
import MultiSelect from '../../src/components/ui/MultiSelect.vue';

const options = [
  { value: 'p1', label: 'flask', hint: '3 open' },
  { value: 'p2', label: 'amafu' },
  { value: 'p3', label: 'cinder' },
];

let wrapper;

function multiSelect(props = {}) {
  wrapper = mount(MultiSelect, {
    props: { label: 'Projects', none: 'All projects', noun: 'projects', options, modelValue: [], ...props },
    attachTo: document.body,
  });
  return wrapper;
}

const panel = (w) => w.find('.multi-select__dropdown');
const toggle = (w) => w.find('.multi-select__toggle');

afterEach(() => wrapper?.unmount());

describe('MultiSelect', () => {
  it('summarises the selection on its toggle', () => {
    expect(toggle(multiSelect()).text()).toBe('All projects');
    expect(toggle(multiSelect({ modelValue: ['p2'] })).text()).toBe('amafu');
    expect(toggle(multiSelect({ modelValue: ['p1', 'p3'] })).text()).toBe('2 projects');
  });

  it('opens a panel of checkboxes reflecting the selection', async () => {
    const w = multiSelect({ modelValue: ['p3'] });
    expect(panel(w).attributes('aria-hidden')).toBe('true');
    await toggle(w).trigger('click');
    expect(panel(w).attributes('aria-hidden')).toBe('false');
    expect(toggle(w).attributes('aria-expanded')).toBe('true');
    expect(w.findAll('.p-checkbox__input').map((input) => input.element.checked)).toEqual([false, false, true]);
    expect(w.find('.multi-select__hint').text()).toBe('3 open');
  });

  it('adds and removes values as boxes are ticked', async () => {
    const w = multiSelect({ modelValue: ['p1'] });
    const [flask, amafu] = w.findAll('.p-checkbox__input');
    await amafu.trigger('change');
    await flask.trigger('change');
    expect(w.emitted('update:modelValue')).toEqual([[['p1', 'p2']], [[]]]);
  });

  it('selects every option or clears the selection', async () => {
    const w = multiSelect({ modelValue: ['p2'] });
    const [selectAll, clear] = w.findAll('.multi-select__actions button');
    await selectAll.trigger('click');
    await clear.trigger('click');
    expect(w.emitted('update:modelValue')).toEqual([[['p1', 'p2', 'p3']], [[]]]);
  });

  it('closes on Escape and returns focus to the toggle', async () => {
    const w = multiSelect();
    await toggle(w).trigger('click');
    await w.find('.multi-select').trigger('keydown', { key: 'Escape' });
    expect(panel(w).attributes('aria-hidden')).toBe('true');
    expect(document.activeElement).toBe(toggle(w).element);
  });

  it('offers a search once there are many options', async () => {
    const many = Array.from({ length: 9 }, (_, i) => ({ value: `p${i}`, label: `project-${i}` }));
    const w = multiSelect({ options: many });
    await w.find('input[type="search"]').setValue('project-4');
    expect(w.findAll('.multi-select__option').map((option) => option.text())).toEqual(['project-4']);
    expect(multiSelect().find('input[type="search"]').exists()).toBe(false);
  });
});

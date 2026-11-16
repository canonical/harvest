import { describe, it, expect, afterEach } from 'vitest';
import { mount } from '@vue/test-utils';
import { h, nextTick } from 'vue';
import AppModal from '../../src/components/ui/AppModal.vue';

afterEach(() => { document.body.innerHTML = ''; });

function open(props = {}, slots = {}) {
  return mount(AppModal, { props, slots, attachTo: document.body });
}

describe('AppModal', () => {
  it('is a labelled modal dialog', () => {
    const wrapper = open({ title: 'Rename chat' });
    const dialog = wrapper.find('[role="dialog"]');
    expect(dialog.attributes('aria-modal')).toBe('true');
    expect(document.getElementById(dialog.attributes('aria-labelledby')).textContent).toBe('Rename chat');
  });

  it('takes its label from a heading in its content when it has no title', async () => {
    const wrapper = open({}, { default: () => h('h3', 'Delete agent') });
    await nextTick();
    const id = wrapper.find('[role="dialog"]').attributes('aria-labelledby');
    expect(document.getElementById(id).textContent).toBe('Delete agent');
  });

  it('moves focus to the first field and closes on Escape', async () => {
    const wrapper = open({ title: 'x' }, { default: () => h('input', { id: 'first' }) });
    expect(document.activeElement.id).toBe('first');
    await wrapper.find('[role="dialog"]').trigger('keydown', { key: 'Escape' });
    expect(wrapper.emitted('close')).toHaveLength(1);
  });

  it('cannot be dismissed when it is not dismissible', async () => {
    const wrapper = open({ title: 'x', dismissible: false });
    expect(wrapper.find('.modal-close').exists()).toBe(false);
    await wrapper.find('[role="dialog"]').trigger('keydown', { key: 'Escape' });
    expect(wrapper.emitted('close')).toBeUndefined();
  });

  it('keeps Tab focus inside the dialog', async () => {
    const wrapper = open({ title: 'x' }, { default: () => [h('input', { id: 'a' }), h('button', { id: 'b' }, 'b')] });
    document.getElementById('b').focus();
    await wrapper.find('[role="dialog"]').trigger('keydown', { key: 'Tab' });
    expect(document.activeElement.classList.contains('modal-close')).toBe(true);
    await wrapper.find('[role="dialog"]').trigger('keydown', { key: 'Tab', shiftKey: true });
    expect(document.activeElement.id).toBe('b');
  });
});

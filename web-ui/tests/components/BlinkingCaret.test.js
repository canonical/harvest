import { describe, it, expect } from 'vitest';
import { mount } from '@vue/test-utils';
import BlinkingCaret from '../../src/components/deployment/BlinkingCaret.vue';

describe('BlinkingCaret', () => {
  it('renders a caret glyph', () => {
    const w = mount(BlinkingCaret);
    expect(w.text()).toBe('▋');
  });

  it('has the blinking-caret class for the CSS animation', () => {
    const w = mount(BlinkingCaret);
    expect(w.classes()).toContain('blinking-caret');
  });

  it('is an inline element (span)', () => {
    const w = mount(BlinkingCaret);
    expect(w.element.tagName).toBe('SPAN');
  });

  it('marks itself aria-hidden since it carries no information of its own', () => {
    const w = mount(BlinkingCaret);
    expect(w.attributes('aria-hidden')).toBe('true');
  });
});

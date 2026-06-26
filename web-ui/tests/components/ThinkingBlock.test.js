import { describe, it, expect } from 'vitest';
import { mount } from '@vue/test-utils';
import ThinkingBlock from '../../src/components/chat/ThinkingBlock.vue';

describe('ThinkingBlock', () => {
  it('renders the thinking text', () => {
    const w = mount(ThinkingBlock, { props: { text: 'I need to search for this.' } });
    expect(w.text()).toContain('I need to search for this.');
  });

  it('has the .thinking-group class', () => {
    const w = mount(ThinkingBlock, { props: { text: 'test' } });
    expect(w.find('.thinking-group').exists()).toBe(true);
  });

  it('text is always visible without user interaction (no details collapse)', () => {
    const w = mount(ThinkingBlock, { props: { text: 'reasoning here' } });
    expect(w.text()).toContain('reasoning here');
    expect(w.find('details').exists()).toBe(false);
  });

  it('uses .thinking-row layout', () => {
    const w = mount(ThinkingBlock, { props: { text: 'test' } });
    expect(w.find('.thinking-row').exists()).toBe(true);
  });

  it('shows live badge when streaming prop is true', () => {
    const w = mount(ThinkingBlock, { props: { text: 'thinking...', streaming: true } });
    expect(w.find('.thinking-badge--live').exists()).toBe(true);
  });

  it('does not show live badge when streaming is false', () => {
    const w = mount(ThinkingBlock, { props: { text: 'done thinking', streaming: false } });
    expect(w.find('.thinking-badge--live').exists()).toBe(false);
  });

  it('shows blinking cursor when streaming', () => {
    const w = mount(ThinkingBlock, { props: { text: 'in progress', streaming: true } });
    expect(w.find('.thinking-cursor').exists()).toBe(true);
  });

  it('hides cursor when not streaming', () => {
    const w = mount(ThinkingBlock, { props: { text: 'done', streaming: false } });
    expect(w.find('.thinking-cursor').exists()).toBe(false);
  });

  it('streaming defaults to false', () => {
    const w = mount(ThinkingBlock, { props: { text: 'test' } });
    expect(w.find('.thinking-badge--live').exists()).toBe(false);
    expect(w.find('.thinking-cursor').exists()).toBe(false);
  });
});

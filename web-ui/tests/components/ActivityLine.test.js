import { describe, it, expect } from 'vitest';
import { mount } from '@vue/test-utils';
import ActivityLine from '../../src/components/chat/ActivityLine.vue';

const working = text => ({ state: 'working', text });

function lineKey(wrapper) {
  return wrapper.find('.activity-line__line').element.__vnode?.key;
}

describe('ActivityLine', () => {
  it('announces the current activity politely', () => {
    const w = mount(ActivityLine, { props: { activity: working('Reading support-matrix.ini'), steps: 3 } });
    const status = w.find('[role="status"]');
    expect(status.attributes('aria-live')).toBe('polite');
    expect(status.text()).toBe('Reading support-matrix.ini');
  });

  it('shimmers only while working', async () => {
    const w = mount(ActivityLine, { props: { activity: working('Thinking') } });
    expect(w.find('.activity-line__line--shimmer').exists()).toBe(true);
    await w.setProps({ activity: { state: 'done', text: 'Worked for 11s · 4 steps' } });
    expect(w.find('.activity-line__line--shimmer').exists()).toBe(false);
    expect(w.classes()).toContain('activity-line--done');
  });

  it('counts steps while working and offers to show them when settled', async () => {
    const w = mount(ActivityLine, { props: { activity: working('Thinking'), steps: 1 } });
    expect(w.find('.activity-line__toggle').text()).toBe('1 step');
    await w.setProps({ activity: { state: 'done', text: 'Worked for 3s · 1 step' }, expanded: true });
    expect(w.find('.activity-line__toggle').text()).toBe('Hide steps');
  });

  it('hides the toggle when there are no steps', () => {
    const w = mount(ActivityLine, { props: { activity: working('Thinking'), steps: 0 } });
    expect(w.find('.activity-line__toggle').exists()).toBe(false);
  });

  it('emits toggle and reflects the expanded state', async () => {
    const w = mount(ActivityLine, { props: { activity: working('Thinking'), steps: 2, expanded: false } });
    const toggle = w.find('.activity-line__toggle');
    expect(toggle.attributes('aria-expanded')).toBe('false');
    await toggle.trigger('click');
    expect(w.emitted('toggle')).toHaveLength(1);
    await w.setProps({ expanded: true });
    expect(toggle.attributes('aria-expanded')).toBe('true');
  });

  it('slides to a new line only when the activity changes, not while a sentence grows', async () => {
    const w = mount(ActivityLine, { props: { activity: working("I'm read") } });
    const first = lineKey(w);
    await w.setProps({ activity: working("I'm reading the matrix") });
    expect(lineKey(w)).toBe(first);
    await w.setProps({ activity: working('Reviewing the results') });
    expect(lineKey(w)).not.toBe(first);
  });
});

import { describe, it, expect } from 'vitest';
import { mount } from '@vue/test-utils';
import RaccoonMark from '../../src/components/chat/RaccoonMark.vue';
import ActivityLine from '../../src/components/chat/ActivityLine.vue';
import { ACTIVITY_STATE } from '../../src/lib/agent-activity.js';
import { RAYS, SWEEP_SECONDS, rayDelay } from '../../src/lib/raccoon-mark.js';

describe('RaccoonMark', () => {
  it('draws every ray around a disc whose face is cut out by its own mask', () => {
    const w = mount(RaccoonMark);
    expect(w.findAll('.raccoon__ray')).toHaveLength(RAYS.length);
    const maskId = w.find('mask').attributes('id');
    expect(w.find('circle[mask]').attributes('mask')).toBe(`url(#${maskId})`);
    expect(w.findAll('.raccoon__eye')).toHaveLength(2);
  });

  it('gives each mask a distinct id so several marks can share a page', () => {
    const first = mount(RaccoonMark);
    const second = mount(RaccoonMark);
    expect(first.find('mask').attributes('id')).not.toBe(second.find('mask').attributes('id'));
  });

  it('spreads the ray sweep evenly over one cycle', () => {
    expect(rayDelay(0)).toBe(`-${SWEEP_SECONDS.toFixed(3)}s`);
    expect(rayDelay(1)).toBe('-1.475s');
    expect(rayDelay(RAYS.length - 1)).toBe('-0.025s');
  });

  it('replaces the status line spinner only while the agent is working', () => {
    const line = state => mount(ActivityLine, { props: { activity: { state, text: 'Searching amafu' } } });
    const working = line(ACTIVITY_STATE.WORKING);
    expect(working.find('.raccoon').classes()).toContain('raccoon--animated');
    expect(working.find('.activity-line__icon').exists()).toBe(false);
    const done = line(ACTIVITY_STATE.DONE);
    expect(done.find('.raccoon').exists()).toBe(false);
    expect(done.find('.activity-line__icon').exists()).toBe(true);
  });
});

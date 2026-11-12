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

  it('sweeps while the agent works and settles once when it finishes live', async () => {
    const line = mount(ActivityLine, { props: { activity: { state: ACTIVITY_STATE.WORKING, text: 'Searching amafu' } } });
    expect(line.find('.raccoon').classes()).toContain('raccoon--working');
    expect(line.find('.activity-line__icon').exists()).toBe(false);
    await line.setProps({ activity: { state: ACTIVITY_STATE.DONE, text: 'Worked for 20s · 2 steps' } });
    expect(line.find('.raccoon').classes()).toContain('raccoon--settling');
    expect(line.find('.activity-line__raccoon').classes()).toContain('activity-line__raccoon--rest');
    expect(line.find('.raccoon__ray').attributes('style')).toBeUndefined();
  });

  it('rests without replaying the settle on answers that were already complete', () => {
    const line = mount(ActivityLine, { props: { activity: { state: ACTIVITY_STATE.DONE, text: 'Worked for 20s' } } });
    expect(line.find('.raccoon').classes()).toContain('raccoon--resting');
  });

  it('keeps the plain icon when the agent waits or is stopped', () => {
    for (const state of [ACTIVITY_STATE.WAITING, ACTIVITY_STATE.STOPPED]) {
      const line = mount(ActivityLine, { props: { activity: { state, text: 'x' } } });
      expect(line.find('.raccoon').exists()).toBe(false);
      expect(line.find('.activity-line__icon').exists()).toBe(true);
    }
  });
});

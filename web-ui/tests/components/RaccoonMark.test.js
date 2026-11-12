import { describe, it, expect } from 'vitest';
import { mount } from '@vue/test-utils';
import RaccoonMark from '../../src/components/chat/RaccoonMark.vue';
import ToolCallStep from '../../src/components/chat/ToolCallStep.vue';
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

  it('animates only while a tool call is running', () => {
    const step = status => ({ type: 'tool_call', id: 1, name: 'search_code', status, input: { repo: 'amafu', query: 'q' } });
    expect(mount(ToolCallStep, { props: { step: step('running') } }).find('.raccoon').classes()).toContain('raccoon--animated');
    expect(mount(ToolCallStep, { props: { step: step('done') } }).find('.raccoon').classes()).not.toContain('raccoon--animated');
  });
});

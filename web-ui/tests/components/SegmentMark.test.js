import { describe, it, expect, vi, afterEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import SegmentMark from '../../src/components/chat/SegmentMark.vue';
import ActivityLine from '../../src/components/chat/ActivityLine.vue';
import ToolCallStep from '../../src/components/chat/ToolCallStep.vue';
import { ACTIVITY_STATE } from '../../src/lib/agent-activity.js';
import { REST_DASH } from '../../src/lib/segment-mark.js';

afterEach(() => vi.unstubAllGlobals());

function frames() {
  let now = 0;
  vi.stubGlobal('requestAnimationFrame', callback => setTimeout(() => callback((now += 16)), 0));
}

describe('SegmentMark', () => {
  it('draws three segments of the ring in orange, aubergine and the text colour', () => {
    const w = mount(SegmentMark);
    expect(w.findAll('.segment-mark__segment').map(s => s.classes().at(-1))).toEqual([
      'segment-mark__segment--0', 'segment-mark__segment--1', 'segment-mark__segment--2',
    ]);
  });

  it('spins while active and rests without animating when it starts inactive', () => {
    expect(mount(SegmentMark, { props: { active: true } }).classes()).toContain('segment-mark--spinning');
    const resting = mount(SegmentMark);
    expect(resting.classes()).not.toContain('segment-mark--spinning');
    expect(resting.classes()).not.toContain('segment-mark--settling');
  });

  it('settles into the resting ring when it stops', async () => {
    frames();
    const w = mount(SegmentMark, { props: { active: true } });
    await w.setProps({ active: false });
    await vi.waitFor(async () => {
      await flushPromises();
      expect(w.find('.segment-mark__segment').element.style.strokeDasharray).toBe(REST_DASH);
    });
    expect(w.classes()).toContain('segment-mark--settling');
    expect(w.classes()).not.toContain('segment-mark--spinning');
    expect(w.find('.segment-mark__turn').element.style.transitionDuration).toMatch(/s$/);
  });

  it('spins again from a clean state when it becomes active after settling', async () => {
    frames();
    const w = mount(SegmentMark, { props: { active: true } });
    await w.setProps({ active: false });
    await vi.waitFor(async () => {
      await flushPromises();
      expect(w.classes()).toContain('segment-mark--settling');
    });
    await w.setProps({ active: true });
    expect(w.classes()).toEqual(expect.arrayContaining(['segment-mark--spinning']));
    expect(w.classes()).not.toContain('segment-mark--settling');
    expect(w.find('.segment-mark__turn').element.style.transform).toBe('');
  });
});

describe('where the segment mark appears', () => {
  const line = state => mount(ActivityLine, { props: { activity: { state, text: 'x' } } });

  it('spins on the status line while the agent works and rests once the answer is done', () => {
    expect(line(ACTIVITY_STATE.WORKING).find('.segment-mark').classes()).toContain('segment-mark--spinning');
    const done = line(ACTIVITY_STATE.DONE);
    expect(done.find('.segment-mark').exists()).toBe(true);
    expect(done.find('.segment-mark').classes()).not.toContain('segment-mark--spinning');
  });

  it('keeps the plain icon while the agent waits or after it stopped', () => {
    for (const state of [ACTIVITY_STATE.WAITING, ACTIVITY_STATE.STOPPED]) {
      expect(line(state).find('.segment-mark').exists()).toBe(false);
      expect(line(state).find('.activity-line__icon').exists()).toBe(true);
    }
  });

  it('spins beside a running tool call and rests beside a finished one', () => {
    const step = status => ({ type: 'tool_call', id: 1, name: 'search_code', status, input: { repo: 'amafu', query: 'q' } });
    expect(mount(ToolCallStep, { props: { step: step('running') } }).find('.segment-mark').classes()).toContain('segment-mark--spinning');
    expect(mount(ToolCallStep, { props: { step: step('done') } }).find('.segment-mark').classes()).not.toContain('segment-mark--spinning');
  });
});

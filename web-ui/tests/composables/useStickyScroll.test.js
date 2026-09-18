import { describe, it, expect } from 'vitest';
import { useStickyScroll } from '../../src/composables/useStickyScroll.js';

function fakeEl({ scrollTop = 0, scrollHeight = 100, clientHeight = 100 } = {}) {
  return { scrollTop, scrollHeight, clientHeight };
}

describe('useStickyScroll', () => {
  it('starts stuck', () => {
    const { stuck } = useStickyScroll();
    expect(stuck.value).toBe(true);
  });

  it('follow scrolls the element to the bottom while stuck', () => {
    const { follow } = useStickyScroll();
    const el = fakeEl({ scrollHeight: 500 });
    follow(el);
    expect(el.scrollTop).toBe(500);
  });

  it('follow does nothing once unstuck', () => {
    const { handleScroll, follow } = useStickyScroll();
    const el = fakeEl({ scrollTop: 0, scrollHeight: 500, clientHeight: 100 });
    handleScroll(el);
    el.scrollTop = 0;
    follow(el);
    expect(el.scrollTop).toBe(0);
  });

  it('follow does nothing when passed a falsy element', () => {
    const { follow } = useStickyScroll();
    expect(() => follow(null)).not.toThrow();
  });

  it('handleScroll unsticks when far from the bottom', () => {
    const { stuck, handleScroll } = useStickyScroll();
    handleScroll(fakeEl({ scrollTop: 0, scrollHeight: 500, clientHeight: 100 }));
    expect(stuck.value).toBe(false);
  });

  it('handleScroll re-sticks when scrolled back within the threshold', () => {
    const { stuck, handleScroll } = useStickyScroll();
    handleScroll(fakeEl({ scrollTop: 0, scrollHeight: 500, clientHeight: 100 }));
    expect(stuck.value).toBe(false);
    handleScroll(fakeEl({ scrollTop: 390, scrollHeight: 500, clientHeight: 100 }));
    expect(stuck.value).toBe(true);
  });

  it('respects a custom threshold', () => {
    const { stuck, handleScroll } = useStickyScroll(50);
    handleScroll(fakeEl({ scrollTop: 420, scrollHeight: 500, clientHeight: 100 }));
    expect(stuck.value).toBe(true);
  });

  it('jumpToLatest re-sticks and scrolls to the bottom even if unstuck', () => {
    const { stuck, handleScroll, jumpToLatest } = useStickyScroll();
    const el = fakeEl({ scrollTop: 0, scrollHeight: 500, clientHeight: 100 });
    handleScroll(el);
    expect(stuck.value).toBe(false);
    jumpToLatest(el);
    expect(stuck.value).toBe(true);
    expect(el.scrollTop).toBe(500);
  });
});

import { describe, it, expect } from 'vitest';
import { angleOf, forwardDelta, settlePlan } from '../../src/lib/segment-mark.js';

describe('segment mark settling', () => {
  it('reads the rotation out of a computed transform', () => {
    expect(angleOf('none')).toBe(0);
    expect(angleOf('matrix(0, 1, -1, 0, 0, 0)')).toBeCloseTo(90);
    expect(angleOf('matrix(-1, 0, 0, -1, 0, 0)')).toBeCloseTo(180);
  });

  it('measures forward turning across the 180° wrap', () => {
    expect(forwardDelta(170, -175)).toBe(15);
    expect(forwardDelta(-10, 5)).toBe(15);
  });

  it('stops on the next third of the ring far enough ahead to brake from the current speed', () => {
    const plan = settlePlan(-170, 290);
    expect(plan.target).toBeCloseTo(0);
    expect(plan.seconds).toBeCloseTo(1.41, 2);
    expect(settlePlan(100, 0).target).toBe(240);
    expect(settlePlan(110, 0).target).toBe(240);
  });

  it('keeps the settle between a short and a long duration', () => {
    expect(settlePlan(0, 5000).seconds).toBeGreaterThanOrEqual(0.6);
    expect(settlePlan(0, 0).seconds).toBe(2.4);
  });
});

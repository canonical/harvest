import { describe, it, expect } from 'vitest';
import { formatDuration, relativeTime } from '../../src/lib/utils.js';

describe('formatDuration', () => {
  it('shows one decimal place under 10 seconds', () => {
    expect(formatDuration(4200)).toBe('4.2s');
  });

  it('drops the decimal between 10 and 60 seconds', () => {
    expect(formatDuration(12345)).toBe('12s');
  });

  it('switches to minutes and seconds at 60 seconds', () => {
    expect(formatDuration(65000)).toBe('1m 05s');
  });

  it('pads seconds under ten when in minutes', () => {
    expect(formatDuration(60000)).toBe('1m 00s');
  });

  it('treats missing input as zero', () => {
    expect(formatDuration(undefined)).toBe('0.0s');
    expect(formatDuration(null)).toBe('0.0s');
  });

  it('clamps negative durations to zero', () => {
    expect(formatDuration(-500)).toBe('0.0s');
  });
});

describe('relativeTime', () => {
  const now = Date.parse('2026-11-17T12:00:00Z');

  it('shows the largest whole unit since the moment', () => {
    expect(relativeTime('2026-11-17T11:59:30Z', now)).toBe('just now');
    expect(relativeTime('2026-11-17T11:45:00Z', now)).toBe('15m ago');
    expect(relativeTime('2026-11-17T09:00:00Z', now)).toBe('3h ago');
    expect(relativeTime('2026-11-15T12:00:00Z', now)).toBe('2d ago');
  });

  it('shows a dash when there is no moment', () => {
    expect(relativeTime(null, now)).toBe('—');
  });
});

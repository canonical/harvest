import { describe, it, expect } from 'vitest';
import { formatDuration } from '../../src/lib/utils.js';

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

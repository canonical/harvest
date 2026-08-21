import { describe, it, expect } from 'vitest';
import { isValidTransition, nextStatusOptions, ISSUE_STATUSES } from '../../src/lib/issue-transitions.js';

describe('isValidTransition', () => {
  it('allows untriaged to in_progress', () => {
    expect(isValidTransition('untriaged', 'in_progress')).toBe(true);
  });

  it('allows in_progress to fixed', () => {
    expect(isValidTransition('in_progress', 'fixed')).toBe(true);
  });

  it('allows in_progress to rejected', () => {
    expect(isValidTransition('in_progress', 'rejected')).toBe(true);
  });

  it('rejects no-op moves', () => {
    expect(isValidTransition('untriaged', 'untriaged')).toBe(false);
    expect(isValidTransition('in_progress', 'in_progress')).toBe(false);
    expect(isValidTransition('fixed', 'fixed')).toBe(false);
  });

  it('rejects backward moves', () => {
    expect(isValidTransition('fixed', 'in_progress')).toBe(false);
    expect(isValidTransition('rejected', 'in_progress')).toBe(false);
    expect(isValidTransition('in_progress', 'untriaged')).toBe(false);
  });

  it('rejects skipping straight from untriaged to fixed or rejected', () => {
    expect(isValidTransition('untriaged', 'fixed')).toBe(false);
    expect(isValidTransition('untriaged', 'rejected')).toBe(false);
  });

  it('rejects moves out of terminal states', () => {
    expect(isValidTransition('fixed', 'rejected')).toBe(false);
    expect(isValidTransition('rejected', 'fixed')).toBe(false);
  });
});

describe('nextStatusOptions', () => {
  it('returns in_progress for untriaged', () => {
    expect(nextStatusOptions('untriaged')).toEqual(['in_progress']);
  });

  it('returns fixed and rejected for in_progress', () => {
    expect(nextStatusOptions('in_progress')).toEqual(['fixed', 'rejected']);
  });

  it('returns an empty array for terminal statuses', () => {
    expect(nextStatusOptions('fixed')).toEqual([]);
    expect(nextStatusOptions('rejected')).toEqual([]);
  });

  it('returns an empty array for an unknown status', () => {
    expect(nextStatusOptions('bogus')).toEqual([]);
  });
});

describe('ISSUE_STATUSES', () => {
  it('lists all four statuses in kanban column order', () => {
    expect(ISSUE_STATUSES).toEqual(['untriaged', 'in_progress', 'fixed', 'rejected']);
  });
});

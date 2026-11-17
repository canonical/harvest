import { describe, it, expect } from 'vitest';
import { selectionSummary, matchingOptions, toggled } from '../../src/lib/multi-select.js';

describe('selection summary', () => {
  const options = [{ value: 'p1', label: 'flask' }, { value: 'p2', label: 'amafu' }, { value: 'p3', label: 'cinder' }];
  const words = { none: 'All projects', noun: 'projects' };

  it('names the choice when there is one', () => {
    expect(selectionSummary(['p2'], options, words)).toBe('amafu');
  });

  it('counts the choices when there are several', () => {
    expect(selectionSummary(['p1', 'p3'], options, words)).toBe('2 projects');
  });

  it('reads as no filter when nothing or everything is chosen', () => {
    expect(selectionSummary([], options, words)).toBe('All projects');
    expect(selectionSummary(['p1', 'p2', 'p3'], options, words)).toBe('All projects');
  });
});

describe('multi-select options', () => {
  const options = [{ value: 'p1', label: 'Flask' }, { value: 'p2', label: 'amafu' }];

  it('matches labels without regard to case or spaces around the search', () => {
    expect(matchingOptions(options, ' fla ')).toEqual([options[0]]);
    expect(matchingOptions(options, '')).toBe(options);
  });

  it('adds a value that is not selected and removes one that is', () => {
    expect(toggled(['p1'], 'p2')).toEqual(['p1', 'p2']);
    expect(toggled(['p1', 'p2'], 'p1')).toEqual(['p2']);
  });
});

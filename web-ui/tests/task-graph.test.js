import { describe, it, expect, beforeEach } from 'vitest';
import { computeGraphLayout } from '../src/task-graph.js';

function node(id, deps = []) {
  return { id, name: id, depends_on: deps };
}

describe('computeGraphLayout', () => {
  it('single node gets level 0', () => {
    const { levels, maxLevel } = computeGraphLayout([node('a')], 'a');
    expect(levels.get('a')).toBe(0);
    expect(maxLevel).toBe(0);
  });

  it('linear chain: root=0, mid=1, target=2', () => {
    // a ← b ← c (c depends on b, b depends on a)
    const tasks = [node('a'), node('b', ['a']), node('c', ['b'])];
    const { levels, maxLevel } = computeGraphLayout(tasks, 'c');
    expect(levels.get('a')).toBe(0);
    expect(levels.get('b')).toBe(1);
    expect(levels.get('c')).toBe(2);
    expect(maxLevel).toBe(2);
  });

  it('diamond: shared root=0, branches=1, target=2', () => {
    // a ← b, a ← c, b+c ← d
    const tasks = [
      node('a'),
      node('b', ['a']),
      node('c', ['a']),
      node('d', ['b', 'c']),
    ];
    const { levels, maxLevel } = computeGraphLayout(tasks, 'd');
    expect(levels.get('a')).toBe(0);
    expect(levels.get('b')).toBe(1);
    expect(levels.get('c')).toBe(1);
    expect(levels.get('d')).toBe(2);
    expect(maxLevel).toBe(2);
  });

  it('maxLevel equals target level', () => {
    const tasks = [node('x'), node('y', ['x']), node('z', ['y'])];
    const { levels, maxLevel } = computeGraphLayout(tasks, 'z');
    expect(maxLevel).toBe(levels.get('z'));
  });

  it('level is longest path from any root (skewed diamond)', () => {
    // a ← b ← c ← d (and also a ← d directly; longest path wins)
    const tasks = [
      node('a'),
      node('b', ['a']),
      node('c', ['b']),
      node('d', ['a', 'c']),
    ];
    const { levels } = computeGraphLayout(tasks, 'd');
    expect(levels.get('d')).toBe(3); // a(0)→b(1)→c(2)→d(3) longest path
  });

  it('byLevel groups nodes at the same level', () => {
    const tasks = [
      node('a'),
      node('b', ['a']),
      node('c', ['a']),
      node('d', ['b', 'c']),
    ];
    const { byLevel } = computeGraphLayout(tasks, 'd');
    expect(byLevel.get(0)).toEqual(['a']);
    expect(byLevel.get(1).sort()).toEqual(['b', 'c']);
    expect(byLevel.get(2)).toEqual(['d']);
  });

  it('handles node with no deps as root regardless of depends_on being empty array', () => {
    const { levels } = computeGraphLayout([{ id: 'x', name: 'x', depends_on: [] }], 'x');
    expect(levels.get('x')).toBe(0);
  });
});

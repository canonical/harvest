import { describe, it, expect } from 'vitest';
import { createLeaf, createSplit, createTab, layoutRects } from '../../src/lib/pane-layout.js';

describe('layoutRects', () => {
  it('returns a single full-size rect for a single leaf', () => {
    const leaf = createLeaf([createTab(), createTab()]);
    const rects = layoutRects(leaf);
    expect(rects).toEqual([{ x: 0, y: 0, w: 100, h: 100, tabCount: 2 }]);
  });

  it('splits a row into side-by-side rects proportional to sizes', () => {
    const root = createSplit('row', [createLeaf([createTab()]), createLeaf([createTab()])]);
    root.sizes = [0.25, 0.75];
    const rects = layoutRects(root);
    expect(rects).toEqual([
      { x: 0, y: 0, w: 25, h: 100, tabCount: 1 },
      { x: 25, y: 0, w: 75, h: 100, tabCount: 1 },
    ]);
  });

  it('splits a column into stacked rects proportional to sizes', () => {
    const root = createSplit('column', [createLeaf([createTab()]), createLeaf([createTab()])]);
    root.sizes = [0.5, 0.5];
    const rects = layoutRects(root);
    expect(rects).toEqual([
      { x: 0, y: 0, w: 100, h: 50, tabCount: 1 },
      { x: 0, y: 50, w: 100, h: 50, tabCount: 1 },
    ]);
  });

  it('handles nested splits', () => {
    const inner = createSplit('column', [createLeaf([createTab()]), createLeaf([createTab()])]);
    const root = createSplit('row', [createLeaf([createTab()]), inner]);
    const rects = layoutRects(root);
    expect(rects).toHaveLength(3);
    expect(rects[0]).toEqual({ x: 0, y: 0, w: 50, h: 100, tabCount: 1 });
    expect(rects[1]).toEqual({ x: 50, y: 0, w: 50, h: 50, tabCount: 1 });
    expect(rects[2]).toEqual({ x: 50, y: 50, w: 50, h: 50, tabCount: 1 });
  });

  it('falls back to even sizes when the sizes array is missing or mismatched', () => {
    const root = createSplit('row', [createLeaf([createTab()]), createLeaf([createTab()]), createLeaf([createTab()])]);
    root.sizes = [1];
    const rects = layoutRects(root);
    rects.forEach(r => expect(r.w).toBeCloseTo(100 / 3));
  });
});

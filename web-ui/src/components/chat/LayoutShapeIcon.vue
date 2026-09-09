<template>
  <svg class="layout-shape" viewBox="0 0 32 22" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
    <g v-for="(pane, i) in panes" :key="i">
      <rect
        :x="pane.x + GAP / 2" :y="pane.y + GAP / 2"
        :width="pane.w - GAP" :height="pane.h - GAP"
        rx="1" class="layout-shape__pane"
      />
      <rect
        v-for="(seg, j) in pane.tabs" :key="j"
        :x="seg.x" :y="pane.y + GAP / 2 + 1"
        :width="seg.w" :height="Math.min(2.5, pane.h - GAP - 2)"
        rx="0.5" class="layout-shape__tab"
      />
    </g>
  </svg>
</template>

<script setup>
import { computed } from 'vue';
import { layoutRects } from '../../lib/pane-layout.js';

const props = defineProps({
  tree: { type: Object, default: null },
});

const GAP = 1.5;
const W = 32;
const H = 22;
const TAB_SEGMENT_GAP = 0.6;
const MAX_TAB_SEGMENTS = 3;

const panes = computed(() => {
  if (!props.tree?.root) return [];
  return layoutRects(props.tree.root, { x: 0, y: 0, w: W, h: H }).map(r => ({
    ...r,
    tabs: tabSegments(r),
  }));
});

function tabSegments(rect) {
  const n = Math.max(1, Math.min(rect.tabCount ?? 1, MAX_TAB_SEGMENTS));
  const stripX = rect.x + GAP / 2;
  const stripW = Math.max(rect.w - GAP, 1);
  const segW = (stripW - (n - 1) * TAB_SEGMENT_GAP) / n;
  return Array.from({ length: n }, (_, i) => ({
    x: stripX + i * (segW + TAB_SEGMENT_GAP),
    w: Math.max(segW, 0.5),
  }));
}
</script>

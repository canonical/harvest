<template>
  <Pane v-if="node.type === 'leaf'" :node="node" />
  <div v-else class="pane-split" :class="`pane-split--${node.direction}`">
    <template v-for="(child, i) in node.children" :key="child.id">
      <div class="pane-split__child" :style="{ flexBasis: sizePercent(i) }">
        <PaneSplit :node="child" />
      </div>
      <div
        v-if="i < node.children.length - 1"
        class="pane-gutter"
        :class="`pane-gutter--${node.direction}`"
        @pointerdown="onGutterPointerDown($event, i)"
      />
    </template>
  </div>
</template>

<script setup>
import Pane from './Pane.vue';
import { useChatWorkspaceStore } from '../../stores/chatWorkspace.js';

const props = defineProps({
  node: { type: Object, required: true },
});

const workspace = useChatWorkspaceStore();

const MIN_FRACTION = 0.08;

function sizePercent(i) {
  const n = props.node.children.length;
  return `${(props.node.sizes[i] ?? 1 / n) * 100}%`;
}

let dragCtx = null;

function onGutterPointerDown(e, i) {
  const containerEl = e.currentTarget.parentElement;
  const rect = containerEl.getBoundingClientRect();
  const isRow = props.node.direction === 'row';
  dragCtx = {
    i,
    startPos: isRow ? e.clientX : e.clientY,
    containerSize: isRow ? rect.width : rect.height,
    startSizes: [...props.node.sizes],
  };
  e.target.setPointerCapture?.(e.pointerId);
  window.addEventListener('pointermove', onGutterPointerMove);
  window.addEventListener('pointerup', onGutterPointerUp);
}

function onGutterPointerMove(e) {
  if (!dragCtx || !dragCtx.containerSize) return;
  const isRow = props.node.direction === 'row';
  const pos = isRow ? e.clientX : e.clientY;
  const delta = (pos - dragCtx.startPos) / dragCtx.containerSize;
  const { i, startSizes } = dragCtx;
  const sizes = [...startSizes];
  let a = startSizes[i] + delta;
  let b = startSizes[i + 1] - delta;
  if (a < MIN_FRACTION) { b -= (MIN_FRACTION - a); a = MIN_FRACTION; }
  if (b < MIN_FRACTION) { a -= (MIN_FRACTION - b); b = MIN_FRACTION; }
  sizes[i] = Math.max(MIN_FRACTION, a);
  sizes[i + 1] = Math.max(MIN_FRACTION, b);
  workspace.resizeSplit(props.node.id, sizes);
}

function onGutterPointerUp() {
  dragCtx = null;
  window.removeEventListener('pointermove', onGutterPointerMove);
  window.removeEventListener('pointerup', onGutterPointerUp);
}
</script>

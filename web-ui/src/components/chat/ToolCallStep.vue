<template>
  <div class="tc-step" :class="[`tc-step--${step.status}`, { 'tc-step--open': open }]">
    <div
      class="tc-step__row"
      :class="{ 'tc-step__row--clickable': hasDetail }"
      :role="hasDetail ? 'button' : undefined"
      :tabindex="hasDetail ? 0 : undefined"
      :aria-expanded="hasDetail ? String(open) : undefined"
      @click="hasDetail && toggle()"
      @keydown.enter.prevent="hasDetail && toggle()"
      @keydown.space.prevent="hasDetail && toggle()"
    >
      <SegmentMark class="tc-step__icon" :active="step.status === 'running'" />

      <span class="tc-step__label">{{ label }}</span>
      <span v-if="step.gapFill" class="tc-step__gap-badge" title="Made during the merge step to fill a coverage gap between leads">gap-fill</span>

      <svg
        v-if="hasDetail"
        class="tc-step__chevron"
        viewBox="0 0 10 10"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
        xmlns="http://www.w3.org/2000/svg"
        aria-hidden="true"
        :style="open ? 'transform: rotate(180deg)' : ''"
      >
        <polyline points="2,3 5,7 8,3"/>
      </svg>
    </div>

    <div v-if="hasDetail && open && searchView" class="tc-step__detail tc-step__detail--explorer">
      <CodeSearchExplorer :view="searchView" />
    </div>

    <div v-else-if="hasDetail && open" class="tc-step__detail">
      <span class="tc-step__tool-tag">{{ step.name }}</span>

      <div v-if="step.input" class="tc-step__detail-section">
        <div class="tc-step__detail-label">Input</div>
        <div class="tool-data" v-html="renderedInput" />
      </div>

      <div v-if="step.preview" class="tc-step__detail-section">
        <div class="tc-step__detail-label">Result</div>
        <div class="tool-data" v-html="renderedPreview" />
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue';
import CodeSearchExplorer from './CodeSearchExplorer.vue';
import SegmentMark from './SegmentMark.vue';
import { describeToolCall } from '../../lib/tool-render.js';
import { isCodeSearchView } from '../../lib/code-search-view.js';
import { renderJsonToHtml, renderPreviewToHtml } from '../../lib/format.js';

const props = defineProps({
  step: { type: Object, required: true },
});

const open = ref(false);

const searchView = computed(() => (isCodeSearchView(props.step.view) ? props.step.view : null));

const hasDetail = computed(() => !!(props.step.input || props.step.preview || searchView.value));

const label = computed(() =>
  props.step.description || describeToolCall(props.step.name, props.step.input ?? {})
);

const renderedInput = computed(() =>
  props.step.input ? renderJsonToHtml(props.step.input) : ''
);

const renderedPreview = computed(() =>
  props.step.preview ? renderPreviewToHtml(props.step.preview, props.step.input?.file ?? null) : ''
);

function toggle() {
  open.value = !open.value;
}
</script>

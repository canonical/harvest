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
      <svg
        v-if="step.status === 'running'"
        class="tc-step__icon tc-step__icon--spinning"
        viewBox="0 0 16 16"
        fill="currentColor"
        aria-hidden="true"
        xmlns="http://www.w3.org/2000/svg"
      >
        <path d="M.81 7.36a1.92 1.92 0 1 1 3.799.572A1.92 1.92 0 0 1 .81 7.36M8.826 3.033a1.92 1.92 0 1 1 3.755.806 1.92 1.92 0 0 1-3.755-.806M7.04 12.585a4.68 4.68 0 0 1-3.19-2.432 2.76 2.76 0 0 1-1.64.202 6.25 6.25 0 0 0 4.498 3.77c.45.098.908.144 1.364.141a2.74 2.74 0 0 1-.562-1.605 5 5 0 0 1-.47-.076M8.394 12.193a1.92 1.92 0 0 1 3.754.805 1.92 1.92 0 1 1-3.754-.805M12.943 11.89a6.3 6.3 0 0 0 1.22-2.587 6.3 6.3 0 0 0-.905-4.782 2.77 2.77 0 0 1-1.08 1.265 4.7 4.7 0 0 1-.154 4.674c.45.37.77.87.919 1.43M2.56 4.892a2.75 2.75 0 0 1 1.603.41 4.68 4.68 0 0 1 3.77-2.015q.012-.218.057-.433c.088-.411.268-.795.525-1.124A6.31 6.31 0 0 0 2.56 4.892"/>
      </svg>
      <svg
        v-else
        class="tc-step__icon"
        viewBox="0 0 16 16"
        fill="currentColor"
        aria-hidden="true"
        xmlns="http://www.w3.org/2000/svg"
      >
        <path d="M.81 7.36a1.92 1.92 0 1 1 3.799.572A1.92 1.92 0 0 1 .81 7.36M8.826 3.033a1.92 1.92 0 1 1 3.755.806 1.92 1.92 0 0 1-3.755-.806M7.04 12.585a4.68 4.68 0 0 1-3.19-2.432 2.76 2.76 0 0 1-1.64.202 6.25 6.25 0 0 0 4.498 3.77c.45.098.908.144 1.364.141a2.74 2.74 0 0 1-.562-1.605 5 5 0 0 1-.47-.076M8.394 12.193a1.92 1.92 0 0 1 3.754.805 1.92 1.92 0 1 1-3.754-.805M12.943 11.89a6.3 6.3 0 0 0 1.22-2.587 6.3 6.3 0 0 0-.905-4.782 2.77 2.77 0 0 1-1.08 1.265 4.7 4.7 0 0 1-.154 4.674c.45.37.77.87.919 1.43M2.56 4.892a2.75 2.75 0 0 1 1.603.41 4.68 4.68 0 0 1 3.77-2.015q.012-.218.057-.433c.088-.411.268-.795.525-1.124A6.31 6.31 0 0 0 2.56 4.892"/>
      </svg>

      <span class="tc-step__label">{{ label }}</span>

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

    <div v-if="hasDetail && open" class="tc-step__detail">
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
import { describeToolCall } from '../../lib/tool-render.js';
import { renderJsonToHtml, renderPreviewToHtml } from '../../lib/format.js';

const props = defineProps({
  step: { type: Object, required: true },
});

const open = ref(false);

const hasDetail = computed(() => !!(props.step.input || props.step.preview));

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

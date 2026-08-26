<template>
  <div class="thinking-group">
    <div class="thinking-row">
      <svg
        class="thinking-icon"
        viewBox="0 0 16 16"
        fill="currentColor"
        xmlns="http://www.w3.org/2000/svg"
        aria-hidden="true"
      >
        <rect x="1" y="2.5"  width="14" height="1.5" rx="0.75"/>
        <rect x="1" y="7"    width="10" height="1.5" rx="0.75"/>
        <rect x="1" y="11.5" width="12" height="1.5" rx="0.75"/>
      </svg>
      <span ref="bodyRef" class="thinking-text-inline" v-html="renderedText"></span>
      <span v-if="streaming" class="thinking-cursor" aria-hidden="true">▋</span>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, watch, nextTick } from 'vue';
import { renderMarkdown } from '../../lib/markdown.js';

const props = defineProps({
  text:      { type: String,  required: true },
  streaming: { type: Boolean, default: false },
});

const bodyRef = ref(null);

const renderedText = computed(() => renderMarkdown(props.text));

watch(renderedText, () => nextTick(() => {
  if (bodyRef.value) {
    import('../../lib/mermaid.js').then(({ mountMermaidDiagrams }) => {
      if (bodyRef.value) mountMermaidDiagrams(bodyRef.value);
    });
  }
}));
</script>

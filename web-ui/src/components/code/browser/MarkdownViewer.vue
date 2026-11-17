<template>
  <article class="markdown-viewer doc-body" v-html="html" />
</template>

<script setup>
import { computed } from 'vue';
import { renderUntrustedMarkdown } from '../../../lib/markdown.js';

const props = defineProps({
  source: { type: String, required: true },
});

const html = computed(() => renderUntrustedMarkdown(props.source));
</script>

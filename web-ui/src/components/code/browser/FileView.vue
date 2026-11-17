<template>
  <section class="file-view" :aria-label="name">
    <header class="file-view__toolbar">
      <span class="file-view__size">{{ formatSize(file.size) }}</span>
      <div v-if="viewer === 'markdown'" class="p-segmented-control is-dense" role="group" aria-label="Show the file as">
        <div class="p-segmented-control__list">
          <button type="button" class="p-segmented-control__button" :aria-pressed="String(!showSource)" @click="showSource = false">Preview</button>
          <button type="button" class="p-segmented-control__button" :aria-pressed="String(showSource)" @click="showSource = true">Code</button>
        </div>
      </div>
      <a class="p-button is-dense file-view__download" :href="rawUrl" :download="name">Download</a>
    </header>
    <component :is="shown.component" v-bind="shown.props" class="file-view__body" @select-line="emit('select-line', $event)" />
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import CodeViewer from './CodeViewer.vue';
import MarkdownViewer from './MarkdownViewer.vue';
import ImageViewer from './ImageViewer.vue';
import FileNotice from './FileNotice.vue';
import { formatSize, nameOf, viewerFor } from '../../../lib/repository-browser.js';

const props = defineProps({
  file: { type: Object, required: true },
  rawUrl: { type: String, required: true },
  line: { type: Number, default: null },
});
const emit = defineEmits(['select-line']);

const VIEWERS = {
  code: { component: CodeViewer, props: ({ file, line }) => ({ path: file.path, content: file.content, line }) },
  markdown: { component: MarkdownViewer, props: ({ file }) => ({ source: file.content }) },
  image: { component: ImageViewer, props: ({ file, rawUrl }) => ({ src: rawUrl, alt: nameOf(file.path) }) },
  binary: { component: FileNotice, props: () => ({ message: 'This is a binary file, so it is not shown here. Download it to open it.' }) },
  'too-large': {
    component: FileNotice,
    props: ({ file }) => ({ message: `This file is ${formatSize(file.size)}, too large to show here. Download it to open it.` }),
  },
};

const showSource = ref(false);
const name = computed(() => nameOf(props.file.path));
const viewer = computed(() => viewerFor(props.file));
const shown = computed(() => {
  const chosen = viewer.value === 'markdown' && showSource.value ? 'code' : viewer.value;
  const { component, props: propsFor } = VIEWERS[chosen];
  return { component, props: propsFor(props) };
});

watch(() => props.file.path, () => {
  showSource.value = false;
});
</script>

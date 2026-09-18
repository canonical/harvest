<template>
  <div class="doc-hero-wrap">
    <div
      ref="containerRef"
      class="doc-hero doc-body"
      :class="{ 'doc-hero--thinking': thinking }"
      data-testid="document-hero"
      @scroll="onScroll"
    >
      <div
        v-for="(block, idx) in frozenBlocks"
        :key="idx"
        class="doc-hero__block"
        data-testid="document-hero-block"
        v-html="renderBlock(block)"
      />
      <template v-if="liveBlock">
        <div
          :key="frozenBlocks.length"
          class="doc-hero__block doc-hero__block--live"
          data-testid="document-hero-block"
          v-html="renderBlock(liveBlock)"
        />
      </template>
      <BlinkingCaret v-if="running" />
    </div>
    <button
      v-if="!stuck"
      type="button"
      class="jump-latest-btn"
      data-testid="document-hero-jump-latest"
      @click="jump"
    >Jump to latest</button>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { tokenizeDocument, renderBlock } from '../../lib/incremental-markdown.js';
import { useStickyScroll } from '../../composables/useStickyScroll.js';
import BlinkingCaret from './BlinkingCaret.vue';

const props = defineProps({
  text:          { type: String, default: '' },
  repoUrlMap:    { type: Object, default: () => ({}) },
  citationIndex: { type: Object, default: () => ({}) },
  running:       { type: Boolean, default: false },
  thinking:      { type: Boolean, default: false },
});

const containerRef = ref(null);
const { stuck, handleScroll, follow, jumpToLatest } = useStickyScroll();

const tokens        = computed(() => tokenizeDocument(props.text, props.repoUrlMap, props.citationIndex));
const frozenBlocks  = computed(() => tokens.value.slice(0, -1));
const liveBlock      = computed(() => tokens.value.at(-1) ?? null);

function onScroll() {
  handleScroll(containerRef.value);
}

function jump() {
  jumpToLatest(containerRef.value);
}

watch(() => props.text, () => follow(containerRef.value), { flush: 'post' });
</script>

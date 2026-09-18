<template>
  <div class="file-tabs-wrap">
    <div v-if="flatTabs.length" class="file-tabs" role="tablist">
      <button
        v-for="tab in flatTabs"
        :key="tab.tabId"
        type="button"
        class="file-tabs__tab"
        :class="{ 'file-tabs__tab--active': tab.tabId === selectedTabId }"
        role="tab"
        :aria-selected="String(tab.tabId === selectedTabId)"
        data-testid="file-tab"
        @click="selectTab(tab.tabId)"
      >
        <svg
          v-if="tab.status === 'saving'"
          class="file-tabs__icon file-tabs__icon--spinning"
          data-testid="file-tab-saving"
          viewBox="0 0 16 16" fill="currentColor" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"
        >
          <path d="M.81 7.36a1.92 1.92 0 1 1 3.799.572A1.92 1.92 0 0 1 .81 7.36M8.826 3.033a1.92 1.92 0 1 1 3.755.806 1.92 1.92 0 0 1-3.755-.806M7.04 12.585a4.68 4.68 0 0 1-3.19-2.432 2.76 2.76 0 0 1-1.64.202 6.25 6.25 0 0 0 4.498 3.77c.45.098.908.144 1.364.141a2.74 2.74 0 0 1-.562-1.605 5 5 0 0 1-.47-.076M8.394 12.193a1.92 1.92 0 0 1 3.754.805 1.92 1.92 0 1 1-3.754-.805M12.943 11.89a6.3 6.3 0 0 0 1.22-2.587 6.3 6.3 0 0 0-.905-4.782 2.77 2.77 0 0 1-1.08 1.265 4.7 4.7 0 0 1-.154 4.674c.45.37.77.87.919 1.43M2.56 4.892a2.75 2.75 0 0 1 1.603.41 4.68 4.68 0 0 1 3.77-2.015q.012-.218.057-.433c.088-.411.268-.795.525-1.124A6.31 6.31 0 0 0 2.56 4.892"/>
        </svg>
        <svg
          v-else
          class="file-tabs__icon file-tabs__icon--saved"
          data-testid="file-tab-saved"
          viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"
        >
          <path d="M3 8.5l3 3 7-7"/>
        </svg>
        <span class="file-tabs__tab-label">{{ tab.path }}</span>
      </button>
    </div>

    <button
      v-if="!following"
      type="button"
      class="file-tabs__resume"
      data-testid="file-tabs-resume"
      @click="resumeLive"
    >Back to live</button>

    <div v-if="selectedTab" class="file-tabs__content-wrap">
      <pre
        ref="contentRef"
        :key="selectedTab.tabId"
        class="file-tabs__content"
        data-testid="file-tab-content"
        @scroll="onScroll"
      ><code v-html="highlightedSelected"></code><BlinkingCaret v-if="showCaret" /></pre>
      <button
        v-if="showScrollLatest"
        type="button"
        class="jump-latest-btn"
        data-testid="file-tabs-scroll-latest"
        @click="jumpToScrollLatest"
      >Jump to latest</button>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import hljs from 'highlight.js';
import { langFromPath } from '../../lib/format.js';
import { escapeHtml } from '../../lib/utils.js';
import { useStickyScroll } from '../../composables/useStickyScroll.js';
import BlinkingCaret from './BlinkingCaret.vue';

const props = defineProps({
  files:        { type: Array, default: () => [] },
  activeFileId: { type: String, default: null },
  running:      { type: Boolean, default: false },
});

const manualTabId = ref(null);
const contentRef   = ref(null);
const { stuck, handleScroll, follow, jumpToLatest } = useStickyScroll();

const flatTabs = computed(() => props.files.flatMap(f => f.subfiles.map(sf => ({
  tabId:  `${f.id}:${sf.path}`,
  fileId: f.id,
  path:   sf.path,
  text:   sf.text,
  status: f.status,
}))));

const liveTabId = computed(() => {
  const f = props.files.find(f => f.id === props.activeFileId);
  const sf = f?.subfiles?.[0];
  return f && sf ? `${f.id}:${sf.path}` : null;
});

const selectedTabId = computed(() => manualTabId.value ?? liveTabId.value);
const selectedTab    = computed(() => flatTabs.value.find(t => t.tabId === selectedTabId.value) ?? null);
const following      = computed(() => manualTabId.value === null);

const highlightedSelected = computed(() => {
  if (!selectedTab.value) return '';
  const lang = langFromPath(selectedTab.value.path);
  if (lang === 'plaintext') return escapeHtml(selectedTab.value.text);
  return hljs.highlight(selectedTab.value.text, { language: lang, ignoreIllegals: true }).value;
});

const isLiveAndSaving = computed(() => following.value && selectedTab.value?.status === 'saving');
const showCaret        = computed(() => props.running && isLiveAndSaving.value);
const showScrollLatest = computed(() => isLiveAndSaving.value && !stuck.value);

function selectTab(tabId) {
  manualTabId.value = tabId === liveTabId.value ? null : tabId;
}

function resumeLive() {
  manualTabId.value = null;
}

function onScroll() {
  handleScroll(contentRef.value);
}

function jumpToScrollLatest() {
  jumpToLatest(contentRef.value);
}

watch(selectedTabId, () => {
  stuck.value = true;
});

watch(() => [selectedTabId.value, selectedTab.value?.text], () => {
  if (isLiveAndSaving.value) follow(contentRef.value);
}, { flush: 'post' });
</script>

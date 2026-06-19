<template>
  <aside class="repo-source-panel" :class="{ 'is-open': isOpen, 'is-expanded': isExpanded }" aria-label="Symbol source code">
    <div class="repo-source-panel__body">
      <div class="repo-source-panel__head">
        <button
          class="p-button--base has-icon u-no-margin source-panel-expand"
          type="button"
          :aria-label="isExpanded ? 'Shrink source panel' : 'Expand source panel'"
          @click="toggleExpand"
        >
          <i :class="isExpanded ? 'p-icon--chevron-right' : 'p-icon--chevron-left'"></i>
        </button>
        <div class="repo-source-panel__title">
          <span class="source-kind-badge" :class="`source-kind-badge--${kind}`">{{ kind }}</span>
          <span class="source-symbol-name">{{ symbolName }}</span>
        </div>
        <button class="source-panel-close" type="button" aria-label="Close source panel" @click="close">✕</button>
      </div>
      <p class="source-symbol-file">{{ symbolFile }}</p>
      <div class="source-code-block">
        <pre><code ref="codeEl" :class="codeClass" v-html="codeHtml"></code></pre>
      </div>
      <div v-if="sections.length" class="source-relations">
        <div v-for="(sec, si) in sections" :key="si" class="source-relations__section">
          <div class="source-relations__label">{{ sec.label }}</div>
          <div class="source-relations__chips">
            <button
              v-for="(chip, ci) in sec.chips"
              :key="ci"
              type="button"
              class="relation-chip"
              :title="chip.title"
              @click="chip.onClick()"
            >{{ chip.name }}</button>
          </div>
        </div>
      </div>
    </div>
  </aside>
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue';
import hljs from 'highlight.js';
import { fetchSymbolSource } from '../lib/api.js';
import { langFromPath } from '../lib/format.js';

const isOpen     = ref(false);
const isExpanded = ref(false);
const kind       = ref('function');
const symbolName = ref('');
const symbolFile = ref('');
const codeHtml   = ref('// Loading…');
const codeClass  = ref('');
const sections   = ref([]);

function close() {
  isOpen.value    = false;
  isExpanded.value = false;
}

function toggleExpand() {
  isExpanded.value = !isExpanded.value;
}

async function open({ nodeData, repo, version, sections: secs }) {
  kind.value       = (nodeData.kind ?? 'function').toLowerCase();
  symbolName.value = nodeData.label ?? nodeData.name ?? '';
  symbolFile.value = nodeData.file
    ? `${nodeData.file}  ·  line ${nodeData.start_line ?? '?'}`
    : '';
  codeHtml.value   = '// Loading…';
  codeClass.value  = '';
  sections.value   = (secs ?? []).filter(s => s.chips.length > 0);
  isOpen.value     = true;

  if (repo && version && nodeData.file) {
    try {
      const sym = await fetchSymbolSource(repo, version, nodeData.file, nodeData.label ?? nodeData.name);
      if (!sym?.source) {
        codeHtml.value = '// Source not available';
        return;
      }
      const lang = langFromPath(nodeData.file);
      if (lang !== 'plaintext') {
        const result = hljs.highlight(sym.source, { language: lang, ignoreIllegals: true });
        codeHtml.value  = result.value;
        codeClass.value = `hljs language-${lang}`;
      } else {
        codeHtml.value  = sym.source.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
        codeClass.value = '';
      }
    } catch (err) {
      codeHtml.value = `// Error: ${err.message}`;
    }
  }
}

function onOpenEvent(e) { open(e.detail); }
function onCloseEvent()  { close(); }

onMounted(() => {
  window.addEventListener('open-source-panel',  onOpenEvent);
  window.addEventListener('close-source-panel', onCloseEvent);
});

onUnmounted(() => {
  window.removeEventListener('open-source-panel',  onOpenEvent);
  window.removeEventListener('close-source-panel', onCloseEvent);
});
</script>

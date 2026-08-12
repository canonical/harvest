<template>
  <div class="diff-view">
    <p v-if="!changedFiles.length" class="diff-view__empty">No changes.</p>
    <div v-for="f in changedFiles" :key="f.path" class="diff-view__file">
      <div class="diff-view__file-header">
        <span class="diff-view__file-path">{{ f.path }}</span>
        <span class="diff-view__file-status" :class="`diff-view__file-status--${f.status}`">{{ f.status }}</span>
      </div>
      <pre class="diff-view__lines"><span
        v-for="(chunk, i) in f.chunks"
        :key="i"
        class="diff-view__chunk"
        :class="{ 'diff-view__chunk--added': chunk.added, 'diff-view__chunk--removed': chunk.removed }"
      >{{ prefixedChunk(chunk) }}</span></pre>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue';
import { diffLines } from 'diff';

const props = defineProps({
  before: { type: Object, required: true },
  after:  { type: Object, required: true },
});

function prefixedChunk(chunk) {
  const prefix = chunk.added ? '+' : chunk.removed ? '-' : ' ';
  const lines = chunk.value.split('\n');
  if (lines.at(-1) === '') lines.pop();
  return lines.map(line => `${prefix} ${line}`).join('\n') + '\n';
}

const changedFiles = computed(() => {
  const paths = new Set([...Object.keys(props.before), ...Object.keys(props.after)]);
  const result = [];
  for (const path of paths) {
    const beforeContent = props.before[path];
    const afterContent  = props.after[path];
    if (beforeContent === afterContent) continue;
    if (beforeContent === undefined) {
      result.push({ path, status: 'added', chunks: [{ added: true, removed: false, value: afterContent }] });
    } else if (afterContent === undefined) {
      result.push({ path, status: 'removed', chunks: [{ added: false, removed: true, value: beforeContent }] });
    } else {
      result.push({ path, status: 'modified', chunks: diffLines(beforeContent, afterContent) });
    }
  }
  return result.sort((a, b) => a.path.localeCompare(b.path));
});
</script>

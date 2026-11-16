<template>
  <div class="preview-page">
    <header class="preview-page__header">
      <strong>{{ details?.project ?? 'Preview' }}</strong>
      <span v-if="details" class="u-text--muted">{{ details.branch }}</span>
    </header>
    <PreviewPanel :preview-id="id" @loaded="details = $event" />
  </div>
</template>

<script setup>
import { ref } from 'vue';
import PreviewPanel from '../components/code/PreviewPanel.vue';

defineProps({ id: { type: String, required: true } });
const details = ref(null);
</script>

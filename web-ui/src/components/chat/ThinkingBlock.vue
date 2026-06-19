<template>
  <details class="thinking-group">
    <summary>Thinking</summary>
    <pre class="thinking-text">{{ text }}</pre>
  </details>
</template>

<script setup>
defineProps({ text: { type: String, required: true } });
</script>

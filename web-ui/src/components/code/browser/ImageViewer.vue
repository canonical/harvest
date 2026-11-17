<template>
  <div class="image-viewer">
    <img :src="src" :alt="alt" />
  </div>
</template>

<script setup>
defineProps({
  src: { type: String, required: true },
  alt: { type: String, required: true },
});
</script>

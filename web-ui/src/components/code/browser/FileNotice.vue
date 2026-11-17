<template>
  <div class="file-notice">
    <div class="p-notification--information">
      <div class="p-notification__content">
        <p class="p-notification__message">{{ message }}</p>
      </div>
    </div>
  </div>
</template>

<script setup>
defineProps({
  message: { type: String, required: true },
});
</script>

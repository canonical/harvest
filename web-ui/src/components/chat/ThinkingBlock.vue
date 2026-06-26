<template>
  <div class="thinking-group">
    <div class="thinking-row">
      <svg
        class="thinking-icon"
        viewBox="0 0 16 16"
        fill="currentColor"
        xmlns="http://www.w3.org/2000/svg"
        aria-hidden="true"
      >
        <rect x="1" y="2.5"  width="14" height="1.5" rx="0.75"/>
        <rect x="1" y="7"    width="10" height="1.5" rx="0.75"/>
        <rect x="1" y="11.5" width="12" height="1.5" rx="0.75"/>
      </svg>
      <span class="thinking-text-inline">
        {{ text }}<span v-if="streaming" class="thinking-cursor" aria-hidden="true">▋</span>
      </span>
      <span v-if="streaming" class="thinking-badge thinking-badge--live">live</span>
    </div>
  </div>
</template>

<script setup>
defineProps({
  text:      { type: String,  required: true },
  streaming: { type: Boolean, default: false },
});
</script>

<template>
  <div class="toast-host" aria-live="polite" aria-atomic="false">
    <div v-for="toast in toasts" :key="toast.id" :class="`p-notification--${toast.kind} toast`" role="status">
      <div class="p-notification__content">
        <p class="p-notification__message">{{ toast.message }}</p>
        <button class="p-notification__close" type="button" aria-label="Dismiss" @click="dismissToast(toast.id)">Close</button>
      </div>
      <div v-if="toast.action" class="p-notification__actions">
        <button class="p-button--base is-dense u-no-margin--bottom" type="button" @click="runToastAction(toast.id)">{{ toast.action.label }}</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { onMounted, onUnmounted } from 'vue';
import { toasts, dismissToast, runToastAction, flushToasts } from '../../composables/useToasts.js';

onMounted(() => window.addEventListener('pagehide', flushToasts));
onUnmounted(() => window.removeEventListener('pagehide', flushToasts));
</script>

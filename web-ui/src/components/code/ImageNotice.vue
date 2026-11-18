<template>
  <div :class="`p-notification--${notice.tone}`">
    <div class="p-notification__content">
      <h5 class="p-notification__title">{{ notice.title }}</h5>
      <p class="p-notification__message">{{ notice.message }}</p>
    </div>
    <div v-if="notice.log" class="p-notification__actions">
      <button class="p-button--base is-dense u-no-margin--bottom" type="button" @click="showLog = !showLog">{{ showLog ? 'Hide log' : 'Show log' }}</button>
    </div>
    <pre v-if="showLog" class="code-log">{{ notice.log }}</pre>
  </div>
</template>

<script setup>
import { ref } from 'vue';

defineProps({
  notice: { type: Object, required: true },
});

const showLog = ref(false);
</script>

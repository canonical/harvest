<template>
  <div class="menu-button" @keydown.esc="close">
    <button
      ref="trigger"
      :class="buttonClass"
      type="button"
      aria-haspopup="menu"
      :aria-expanded="String(open)"
      :aria-label="label"
      :title="title || label"
      @click="toggle"
    >
      <slot>{{ label }}</slot>
    </button>
    <ContextualMenu v-if="open" :items="items" :label="label" :align="align" @select="choose" @close="close">
      <template v-if="$slots.header" #header><slot name="header" /></template>
    </ContextualMenu>
  </div>
</template>

<script setup>
import { ref } from 'vue';
import ContextualMenu from './ContextualMenu.vue';

defineProps({
  label: { type: String, required: true },
  title: { type: String, default: '' },
  items: { type: Array, required: true },
  align: { type: String, default: 'right' },
  buttonClass: { type: String, default: 'p-button--base is-dense' },
});
const emit = defineEmits(['select', 'open']);

const open = ref(false);
const trigger = ref(null);

function toggle() {
  open.value = !open.value;
  if (open.value) emit('open');
}

function close() {
  if (!open.value) return;
  open.value = false;
  trigger.value?.focus();
}

function choose(item) {
  open.value = false;
  trigger.value?.focus();
  emit('select', item);
}

defineExpose({ close });
</script>

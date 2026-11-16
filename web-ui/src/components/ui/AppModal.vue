<template>
  <div class="modal" v-bind="$attrs" @mousedown.self="dismiss">
    <div
      ref="dialog"
      class="modal-content"
      :class="size === 'default' ? null : `modal-content--${size}`"
      role="dialog"
      aria-modal="true"
      :aria-labelledby="labelId"
      tabindex="-1"
      @keydown="onKeydown"
    >
      <button v-if="dismissible" class="modal-close" type="button" aria-label="Close" @click="dismiss">✕</button>
      <h2 v-if="title" :id="titleId">{{ title }}</h2>
      <slot />
      <div v-if="$slots.actions" class="modal-actions"><slot name="actions" /></div>
    </div>
  </div>
</template>

<script setup>
import { onMounted, onUnmounted, ref } from 'vue';
import { focusables, keepFocusInside } from '../../lib/focus.js';

defineOptions({ inheritAttrs: false });

const props = defineProps({
  title: { type: String, default: '' },
  size: { type: String, default: 'default', validator: (v) => ['default', 'wide', 'xwide'].includes(v) },
  dismissible: { type: Boolean, default: true },
});
const emit = defineEmits(['close']);

const dialog = ref(null);
const titleId = `modal-title-${Math.random().toString(36).slice(2, 10)}`;
const labelId = ref(props.title ? titleId : null);
let returnFocusTo = null;

function dismiss() {
  if (props.dismissible) emit('close');
}

function onKeydown(event) {
  if (event.key === 'Escape') {
    event.stopPropagation();
    dismiss();
    return;
  }
  keepFocusInside(event, dialog.value);
}

function labelFromHeading() {
  const heading = dialog.value.querySelector('h1, h2, h3, h4');
  if (!heading) return;
  heading.id ||= titleId;
  labelId.value = heading.id;
}

onMounted(() => {
  returnFocusTo = document.activeElement;
  if (!props.title) labelFromHeading();
  const first = focusables(dialog.value).find(el => !el.classList.contains('modal-close'));
  (first ?? dialog.value).focus();
});

onUnmounted(() => {
  if (returnFocusTo?.isConnected) returnFocusTo.focus();
});
</script>

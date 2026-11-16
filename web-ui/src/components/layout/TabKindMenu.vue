<template>
  <div ref="root" class="tab-kind-menu p-contextual-menu__dropdown" :aria-label="label" @keydown.esc="emit('close')" @focusout="onFocusOut">
    <span class="tab-kind-menu__label">{{ label }}</span>
    <button
      v-for="(entry, index) in kinds"
      :key="entry.kind"
      :ref="(el) => { if (index === 0) first = el; }"
      class="p-contextual-menu__link tab-kind-menu__item"
      type="button"
      @click="emit('choose', entry.kind)"
    >
      <NavIcon :name="entry.icon" />
      <span>{{ entry.label }}</span>
    </button>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue';
import NavIcon from '../nav/NavIcon.vue';

defineProps({
  kinds: { type: Array, required: true },
  label: { type: String, required: true },
});

const emit = defineEmits(['choose', 'close']);
const root = ref(null);
let first = null;

function onFocusOut(event) {
  if (!root.value?.contains(event.relatedTarget)) emit('close');
}

onMounted(() => first?.focus());
</script>

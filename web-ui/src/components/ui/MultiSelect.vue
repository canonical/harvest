<template>
  <div ref="root" class="p-contextual-menu--left multi-select" @keydown="onKeydown" @focusout="onFocusOut">
    <button
      ref="toggle"
      type="button"
      class="p-contextual-menu__toggle p-button has-icon multi-select__toggle"
      :class="{ 'is-dense': dense }"
      aria-haspopup="true"
      :aria-expanded="String(open)"
      :aria-controls="panelId"
      :aria-label="`${label}: ${summary}`"
      @click="open = !open"
    >
      <span class="multi-select__summary">{{ summary }}</span>
      <i class="p-icon--chevron-down p-contextual-menu__indicator" />
    </button>
    <div :id="panelId" class="p-contextual-menu__dropdown multi-select__dropdown" :aria-hidden="String(!open)">
      <div v-if="searchable" class="p-contextual-menu__group multi-select__search">
        <input v-model="search" type="search" :placeholder="`Search ${noun}`" :aria-label="`Search ${noun}`" />
      </div>
      <fieldset class="p-contextual-menu__group multi-select__options">
        <legend class="u-off-screen">{{ label }}</legend>
        <label v-for="option in visibleOptions" :key="option.value" class="p-checkbox multi-select__option">
          <input
            type="checkbox"
            class="p-checkbox__input"
            :checked="modelValue.includes(option.value)"
            @change="emit('update:modelValue', toggled(modelValue, option.value))"
          />
          <span class="p-checkbox__label">
            {{ option.label }}
            <span v-if="option.hint" class="multi-select__hint">{{ option.hint }}</span>
          </span>
        </label>
        <p v-if="!visibleOptions.length" class="multi-select__empty">No {{ noun }} match.</p>
      </fieldset>
      <div v-if="modelValue.length" class="p-contextual-menu__group multi-select__actions">
        <button type="button" class="p-button--base is-small" @click="clear">Clear</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, nextTick, onUnmounted, ref, watch } from 'vue';
import { matchingOptions, selectionSummary, toggled } from '../../lib/multi-select.js';
import { moveFocus } from '../../lib/focus.js';

const SEARCH_THRESHOLD = 7;

const props = defineProps({
  label: { type: String, required: true },
  options: { type: Array, required: true },
  modelValue: { type: Array, required: true },
  none: { type: String, required: true },
  noun: { type: String, required: true },
  dense: { type: Boolean, default: false },
});
const emit = defineEmits(['update:modelValue']);

const root = ref(null);
const toggle = ref(null);
const open = ref(false);
const search = ref('');
const panelId = `multi-select-${Math.random().toString(36).slice(2, 10)}`;

const searchable = computed(() => props.options.length > SEARCH_THRESHOLD);
const visibleOptions = computed(() => matchingOptions(props.options, search.value));
const summary = computed(() => selectionSummary(props.modelValue, props.options, { none: props.none, noun: props.noun }));

function inputs() {
  return [...root.value.querySelectorAll('.multi-select__dropdown input, .multi-select__actions button')];
}

async function clear() {
  emit('update:modelValue', []);
  await nextTick();
  inputs()[0]?.focus();
}

function close({ refocus }) {
  open.value = false;
  if (refocus) toggle.value.focus();
}

function onKeydown(event) {
  if (!open.value) return;
  if (event.key === 'Escape') {
    event.stopPropagation();
    close({ refocus: true });
  } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault();
    moveFocus(inputs(), document.activeElement, event.key === 'ArrowDown' ? 1 : -1);
  }
}

function onFocusOut(event) {
  if (open.value && event.relatedTarget && !root.value.contains(event.relatedTarget)) close({ refocus: false });
}

function onPointerDown(event) {
  if (!root.value.contains(event.target)) close({ refocus: false });
}

watch(open, async (isOpen) => {
  search.value = '';
  if (!isOpen) {
    document.removeEventListener('pointerdown', onPointerDown);
    return;
  }
  document.addEventListener('pointerdown', onPointerDown);
  await nextTick();
  inputs()[0]?.focus();
});

onUnmounted(() => document.removeEventListener('pointerdown', onPointerDown));
</script>

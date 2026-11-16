<template>
  <div
    ref="root"
    class="app-menu p-contextual-menu__dropdown"
    :class="`app-menu--${align}`"
    role="menu"
    :aria-label="label"
    @keydown="onKeydown"
    @focusout="onFocusOut"
  >
    <slot name="header" />
    <template v-for="item in items" :key="item.id">
      <span v-if="item.divider" class="app-menu__divider" role="separator" />
      <span v-else-if="item.heading" class="app-menu__heading">{{ item.label }}</span>
      <button
        v-else
        class="p-contextual-menu__link app-menu__item"
        :class="{ 'app-menu__item--danger': item.danger, 'is-checked': item.checked }"
        type="button"
        :role="item.checked === undefined ? 'menuitem' : 'menuitemradio'"
        :aria-checked="item.checked === undefined ? undefined : String(item.checked)"
        :disabled="item.disabled"
        @click="choose(item)"
      >
        <NavIcon v-if="item.icon" :name="item.icon" />
        <span class="app-menu__text">
          <span class="app-menu__label">{{ item.label }}</span>
          <span v-if="item.hint" class="app-menu__hint">{{ item.hint }}</span>
        </span>
        <kbd v-if="item.shortcut" class="app-menu__shortcut">{{ item.shortcut }}</kbd>
      </button>
    </template>
    <slot name="footer" />
  </div>
</template>

<script setup>
import { onMounted, ref } from 'vue';
import NavIcon from '../nav/NavIcon.vue';
import { moveFocus } from '../../lib/focus.js';

defineProps({
  items: { type: Array, required: true },
  label: { type: String, required: true },
  align: { type: String, default: 'right' },
});
const emit = defineEmits(['select', 'close']);
const root = ref(null);

function menuItems() {
  return [...root.value.querySelectorAll('.app-menu__item:not([disabled])')];
}

function choose(item) {
  emit('select', item);
}

function onKeydown(event) {
  const steps = { ArrowDown: 1, ArrowUp: -1 };
  if (event.key === 'Escape') {
    event.stopPropagation();
    emit('close');
  } else if (event.key in steps && !event.target.matches('input')) {
    event.preventDefault();
    moveFocus(menuItems(), document.activeElement, steps[event.key]);
  } else if (event.key === 'ArrowDown') {
    event.preventDefault();
    menuItems()[0]?.focus();
  } else if (event.key === 'Home' || event.key === 'End') {
    const items = menuItems();
    if (!items.length || event.target.matches('input')) return;
    event.preventDefault();
    (event.key === 'Home' ? items[0] : items[items.length - 1]).focus();
  }
}

function onFocusOut(event) {
  if (!root.value?.contains(event.relatedTarget)) emit('close');
}

onMounted(() => {
  const input = root.value.querySelector('input');
  (input ?? menuItems()[0])?.focus();
});
</script>

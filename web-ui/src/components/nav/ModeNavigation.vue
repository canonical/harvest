<template>
  <div class="mode-nav">
    <div class="mode-switch" role="tablist" aria-label="Mode">
      <router-link
        v-for="entry in MODES"
        :key="entry.id"
        :to="entry.home"
        role="tab"
        class="mode-switch__item"
        :class="{ 'is-active': entry.id === mode }"
        :aria-selected="String(entry.id === mode)"
        @click="emit('navigate')"
      >
        <NavIcon :name="entry.icon" />
        <span class="mode-switch__label is-fading-when-collapsed">{{ entry.label }}</span>
      </router-link>
    </div>

    <slot />

    <div class="p-side-navigation--icons sidenav-top-container">
      <ul class="p-side-navigation__list">
        <li v-for="page in pages" :key="page.to" class="p-side-navigation__item">
          <router-link
            :to="page.to"
            :exact-active-class="page.exact ? 'is-active' : ''"
            :active-class="page.exact ? '' : 'is-active'"
            class="p-side-navigation__link"
            @click="emit('navigate')"
          >
            <NavIcon :name="page.icon" />
            <span class="p-side-navigation__label">{{ page.label }}</span>
          </router-link>
        </li>
      </ul>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue';
import NavIcon from './NavIcon.vue';
import { MODES, pagesFor } from '../../lib/modes.js';

const props = defineProps({
  mode: { type: String, required: true },
});

const emit = defineEmits(['navigate']);

const pages = computed(() => pagesFor(props.mode));
</script>

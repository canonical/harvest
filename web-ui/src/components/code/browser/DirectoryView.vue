<template>
  <section class="directory-view">
    <p v-if="!entries.length" class="u-text--muted">This directory is empty.</p>
    <ul v-else class="directory-view__entries">
      <li v-for="entry in entries" :key="entry.path">
        <router-link v-if="isNavigable(entry)" :to="linkTo(entry)" class="directory-view__entry">
          <i :class="isDirectory(entry) ? 'p-icon--folder' : 'p-icon--file'" />
          <span>{{ entry.name }}</span>
        </router-link>
        <span v-else class="directory-view__entry is-static">
          <i class="p-icon--repository" />
          <span>{{ entry.name }}</span>
          <span class="p-label">submodule</span>
        </span>
      </li>
    </ul>
    <article v-if="readme" class="directory-view__readme">
      <header class="directory-view__readme-title">
        <i class="p-icon--file" />
        <span>{{ nameOf(readme.path) }}</span>
      </header>
      <MarkdownViewer v-if="isMarkdown(readme.path)" :source="readme.content" />
      <pre v-else class="directory-view__readme-text">{{ readme.content }}</pre>
    </article>
  </section>
</template>

<script setup>
import MarkdownViewer from './MarkdownViewer.vue';
import { isDirectory, isMarkdown, nameOf } from '../../../lib/repository-browser.js';

defineProps({
  entries: { type: Array, required: true },
  readme: { type: Object, default: null },
  linkTo: { type: Function, required: true },
});

const isNavigable = (entry) => entry.kind !== 'submodule';
</script>

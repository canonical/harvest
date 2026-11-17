<template>
  <div class="settings-page code-page code-sessions-page">
    <div class="settings-header">
      <h2>Code repositories</h2>
    </div>

    <input v-model="search" type="search" class="repo-list__search" placeholder="Search repositories" aria-label="Search repositories" />

    <p v-if="error" class="code-error" role="alert">{{ error }}</p>
    <p v-else-if="loaded && !visible.length" class="u-text--muted">{{ search.trim() ? 'No repository matches.' : 'No repository has been ingested yet.' }}</p>

    <ul class="repo-list">
      <li v-for="repository in visible" :key="repository.name">
        <router-link :to="`/code/repositories/${encodeURIComponent(repository.name)}`" class="repo-list__item">
          <i class="p-icon--repository" />
          <span class="repo-list__main">
            <span class="repo-list__name">{{ repository.name }}</span>
            <span class="repo-list__url">{{ repository.url }}</span>
          </span>
          <span v-if="repository.visibility === 'private'" class="p-label">private</span>
        </router-link>
      </li>
    </ul>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import { fetchRepositories } from '../lib/api.js';
import { listCodeProjects } from '../lib/code-api.js';
import { repositoryChoices, searchChoices } from '../lib/code-sessions.js';

const repositories = ref([]);
const search = ref('');
const loaded = ref(false);
const error = ref('');

const visible = computed(() => searchChoices(repositories.value, search.value));

onMounted(async () => {
  try {
    const [ingested, projects] = await Promise.all([fetchRepositories(), listCodeProjects().catch(() => [])]);
    repositories.value = repositoryChoices(ingested, projects);
  } catch (e) {
    error.value = e.message;
  } finally {
    loaded.value = true;
  }
});
</script>

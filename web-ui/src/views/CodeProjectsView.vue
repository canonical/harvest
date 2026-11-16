<template>
  <div class="settings-page code-page">
    <div class="settings-header">
      <h2>Code</h2>
    </div>

    <section class="settings-section">
      <div class="settings-section__heading">
        <h3>Open a repository</h3>
        <p class="settings-section__description">Paste the URL of a git repository. Private repositories use the SSH key or token from your Git access settings.</p>
      </div>
      <form class="code-open" @submit.prevent="open">
        <input v-model="url" type="text" placeholder="https://github.com/owner/repository" aria-label="Repository URL" :disabled="opening" />
        <button class="p-button--positive is-dense" type="submit" :disabled="!url.trim() || opening">{{ opening ? 'Checking…' : 'Open' }}</button>
      </form>
      <p v-if="error" class="code-error" role="alert">{{ error }}</p>
    </section>

    <section class="settings-section">
      <div class="settings-section__heading">
        <h3>Recent repositories</h3>
      </div>
      <p v-if="!loading && !projects.length" class="u-text--muted">No repositories opened yet.</p>
      <ul class="code-projects">
        <li v-for="project in projects" :key="project.id">
          <router-link :to="`/code/projects/${project.id}`" class="code-projects__item">
            <span class="code-projects__name">{{ project.name }}</span>
            <span class="code-projects__meta">
              <span v-if="project.visibility === 'private'" class="p-label">private</span>
              <span v-if="project.open_sessions > 0" class="p-label">{{ project.open_sessions }} open</span>
              <span class="u-text--muted">{{ imageLabel(project.image_status) }}</span>
            </span>
          </router-link>
        </li>
      </ul>
    </section>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { listCodeProjects, openCodeProject } from '../lib/code-api.js';

const router = useRouter();
const url = ref('');
const projects = ref([]);
const loading = ref(true);
const opening = ref(false);
const error = ref('');

const IMAGE_LABELS = { ready: 'Ready', building: 'Preparing containers…', failed: 'Setup failed' };

function imageLabel(status) {
  return IMAGE_LABELS[status] ?? 'Not prepared yet';
}

async function load() {
  try {
    projects.value = await listCodeProjects();
  } catch (e) {
    error.value = e.message;
  } finally {
    loading.value = false;
  }
}

async function open() {
  opening.value = true;
  error.value = '';
  try {
    const project = await openCodeProject(url.value.trim());
    router.push(`/code/projects/${project.id}`);
  } catch (e) {
    error.value = e.message;
  } finally {
    opening.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="settings-page code-page code-sessions-page">
    <div class="settings-header code-sessions-page__header">
      <h2>Sessions</h2>
      <button class="p-button--positive is-dense" type="button" @click="creating = true">New session</button>
    </div>

    <section class="settings-section">
      <div class="code-filters" role="group" aria-label="Filter sessions">
        <MultiSelect
          label="Projects"
          none="All projects"
          noun="projects"
          :options="projectOptions"
          :model-value="filters.projectIds"
          @update:model-value="(projectIds) => applyFilters({ projectIds })"
        />
        <label class="p-checkbox code-filters__closed">
          <input type="checkbox" class="p-checkbox__input" :checked="filters.includeClosed" @change="applyFilters({ includeClosed: $event.target.checked })" />
          <span class="p-checkbox__label">Show closed</span>
        </label>
      </div>

      <p v-if="selectedProject" class="code-filters__project">
        <router-link :to="`/code/projects/${selectedProject.id}`">{{ selectedProject.name }} settings</router-link>
      </p>

      <p v-if="error" class="code-error" role="alert">{{ error }}</p>
      <p v-else-if="loaded && !visibleSessions.length" class="u-text--muted">{{ emptyMessage }}</p>

      <ul class="code-sessions">
        <li v-for="session in visibleSessions" :key="session.id">
          <router-link :to="`/code/sessions/${session.id}`" class="code-sessions__item" :title="session.error ?? undefined">
            <span class="code-sessions__main">
              <span class="code-sessions__branch">{{ session.branch }}</span>
              <span class="code-sessions__context">{{ session.project_name }} · from {{ session.base_branch }}</span>
            </span>
            <span class="code-sessions__meta">
              <span class="code-state" :class="`code-state--${sessionState(session)}`">{{ SESSION_STATES[sessionState(session)] }}</span>
              <time class="code-sessions__time" :datetime="session.last_active_at">{{ relativeTime(session.last_active_at, now) }}</time>
            </span>
          </router-link>
        </li>
      </ul>
    </section>

    <NewSessionModal v-if="creating" :project-id="selectedProject?.id ?? null" :projects="projects" @close="creating = false" />
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { listCodeProjects, listCodeSessions } from '../lib/code-api.js';
import { SESSION_STATES, sessionState, filterSessions, filtersFromQuery, queryFromFilters } from '../lib/code-sessions.js';
import { relativeTime } from '../lib/utils.js';
import NewSessionModal from '../components/code/NewSessionModal.vue';
import MultiSelect from '../components/ui/MultiSelect.vue';

const REFRESH_MS = 5000;

const route = useRoute();
const router = useRouter();
const projects = ref([]);
const sessions = ref([]);
const loaded = ref(false);
const error = ref('');
const now = ref(Date.now());
const creating = ref(false);
let refreshTimer = null;

const filters = computed(() => filtersFromQuery(route.query));
const selectedProject = computed(() => {
  const [only, ...others] = filters.value.projectIds;
  return others.length ? null : projects.value.find((project) => project.id === only) ?? null;
});
const projectOptions = computed(() =>
  projects.value.map((project) => ({
    value: project.id,
    label: project.name,
    hint: project.open_sessions > 0 ? `${project.open_sessions} open` : '',
  })),
);
const visibleSessions = computed(() => filterSessions(sessions.value, filters.value));
const emptyMessage = computed(() => {
  if (!projects.value.length) return 'Start your first session with New session.';
  return filters.value.includeClosed ? 'No sessions yet.' : 'No open sessions.';
});

function applyFilters(changes) {
  router.replace({ query: queryFromFilters({ ...filters.value, ...changes }) });
}

async function refresh() {
  try {
    [projects.value, sessions.value] = await Promise.all([listCodeProjects(), listCodeSessions()]);
    error.value = '';
  } catch (e) {
    error.value = e.message;
  } finally {
    loaded.value = true;
    now.value = Date.now();
  }
}

onMounted(() => {
  refresh();
  refreshTimer = setInterval(refresh, REFRESH_MS);
});

onUnmounted(() => clearInterval(refreshTimer));
</script>

<template>
  <template v-if="!auth.isLoggedIn && isAuthRoute">
    <router-view />
  </template>

  <template v-else-if="auth.isLoggedIn">
    <div class="l-application">
      <div class="l-navigation-bar">
        <div class="p-panel is-dark nav-panel">
          <div class="p-panel__header">
            <a class="p-panel__logo nav-logo" href="#">
              <img src="/canonical-logo.png" alt="Canonical" class="p-panel__logo-image" height="32" />
              <span class="p-heading--4">Harvest</span>
            </a>
            <div class="p-panel__controls">
              <button
                class="p-panel__toggle"
                type="button"
                aria-label="Open navigation"
                @click="navCollapsed = false"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="3" y1="12" x2="21" y2="12"/><line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="18" x2="21" y2="18"/></svg>
              </button>
            </div>
          </div>
        </div>
      </div>

      <nav
        id="app-sidebar"
        class="l-navigation"
        :class="{ 'is-collapsed': navCollapsed }"
        aria-label="Main navigation"
      >
        <div class="l-navigation__drawer">
          <div class="p-panel is-dark nav-panel">

            <div class="p-panel__header is-sticky">
              <a class="p-panel__logo nav-logo" href="#">
                <img src="/canonical-logo.png" alt="Canonical" class="p-panel__logo-image" height="32" />
                <span class="p-heading--4">Harvest</span>
              </a>
              <div class="p-panel__controls u-hide--medium u-hide--large">
                <button
                  class="p-button--base has-icon u-no-margin"
                  type="button"
                  aria-label="Close navigation"
                  @click="closeNav"
                >
                  <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                </button>
              </div>
            </div>

            <div class="p-panel__content">
              <div class="project-selector-section">
                <span class="project-selector__label">Project</span>
                <button
                  class="project-selector__toggle"
                  type="button"
                  :aria-expanded="projectDropdownOpen"
                  @click="toggleProjectDropdown"
                >
                  <svg class="project-selector__folder-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
                  <span class="project-selector__name">{{ project.selectedProject?.name ?? 'Select project' }}</span>
                  <svg class="project-selector__chevron" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="4 6 8 10 12 6"/></svg>
                </button>
                <div v-if="projectDropdownOpen" class="project-selector__dropdown">
                  <input
                    v-model="projectSearch"
                    type="search"
                    class="project-selector__search-input"
                    placeholder="Search projects…"
                    autocomplete="off"
                    @click.stop
                  />
                  <div class="project-selector__list">
                    <button
                      v-for="p in filteredProjects"
                      :key="p.id"
                      type="button"
                      class="project-selector__item"
                      :class="{ 'is-selected': p.id === project.selectedProjectId }"
                      @click="chooseProject(p)"
                    >
                      <span class="project-item__name">{{ p.name }}</span>
                      <span v-if="p.group_name" class="p-chip project-item__group-badge">{{ p.group_name }}</span>
                    </button>
                    <p v-if="!filteredProjects.length" class="project-selector__empty"></p>
                  </div>
                  <div class="project-selector__footer">
                    <button type="button" class="project-selector__new-btn" @click="openCreateProject">+ New project</button>
                  </div>
                </div>
              </div>

              <div class="p-side-navigation--icons sidenav-top-container">
                <ul class="p-side-navigation__list">
                  <li class="p-side-navigation__item">
                    <router-link to="/" exact-active-class="is-active" class="p-side-navigation__link" @click="closeNavMobile">
                      <svg class="p-side-navigation__icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
                      <span class="p-side-navigation__label">Chat</span>
                    </router-link>
                  </li>
                  <li class="p-side-navigation__item">
                    <router-link to="/agents" active-class="is-active" class="p-side-navigation__link" @click="closeNavMobile">
                      <svg class="p-side-navigation__icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="2" width="20" height="8" rx="2" ry="2"/><rect x="2" y="14" width="20" height="8" rx="2" ry="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/></svg>
                      <span class="p-side-navigation__label">Agents</span>
                    </router-link>
                  </li>
                  <li class="p-side-navigation__item">
                    <router-link to="/tasks" active-class="is-active" class="p-side-navigation__link" @click="closeNavMobile">
                      <svg class="p-side-navigation__icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/></svg>
                      <span class="p-side-navigation__label">Tasks</span>
                    </router-link>
                  </li>
                  <li class="p-side-navigation__item">
                    <router-link to="/memories" active-class="is-active" class="p-side-navigation__link" @click="closeNavMobile">
                      <svg class="p-side-navigation__icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="4" y="4" width="16" height="16" rx="2"/><rect x="9" y="9" width="6" height="6"/><path d="M15 2v2M9 2v2M2 15h2M2 9h2M15 22v-2M9 22v-2M22 15h-2M22 9h-2"/></svg>
                      <span class="p-side-navigation__label">Memories</span>
                    </router-link>
                  </li>
                  <li class="sidenav-separator" role="separator" aria-hidden="true"></li>
                  <li class="p-side-navigation__item">
                    <router-link to="/repositories" active-class="is-active" class="p-side-navigation__link" @click="closeNavMobile">
                      <svg class="p-side-navigation__icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>
                      <span class="p-side-navigation__label">Explore code</span>
                    </router-link>
                  </li>
                  <li v-if="auth.features.docs" class="p-side-navigation__item">
                    <router-link to="/docs" active-class="is-active" class="p-side-navigation__link" @click="closeNavMobile">
                      <svg class="p-side-navigation__icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></svg>
                      <span class="p-side-navigation__label">Document</span>
                    </router-link>
                  </li>
                </ul>
              </div>

              <div class="p-side-navigation--icons sidenav-bottom-container">
                <ul class="p-side-navigation__list">
                  <li v-if="auth.isAdmin" class="p-side-navigation__item">
                    <router-link to="/admin" active-class="is-active" class="p-side-navigation__link" @click="closeNavMobile">
                      <svg class="p-side-navigation__icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>
                      <span class="p-side-navigation__label">Admin</span>
                    </router-link>
                  </li>
                  <li class="p-side-navigation__item">
                    <button
                      id="theme-btn"
                      type="button"
                      class="p-side-navigation__link"
                      @click="theme.nextTheme()"
                    >
                      <span class="p-side-navigation__icon" aria-hidden="true" style="font-size:1rem;display:flex;align-items:center;justify-content:center;">{{ theme.icon }}</span>
                      <span class="p-side-navigation__label">{{ theme.label }}</span>
                    </button>
                  </li>
                  <li class="p-side-navigation__item">
                    <button
                      id="logout-btn"
                      type="button"
                      class="p-side-navigation__link"
                      @click="handleLogout"
                    >
                      <svg class="p-side-navigation__icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><polyline points="16 17 21 12 16 7"/><line x1="21" y1="12" x2="9" y2="12"/></svg>
                      <span class="p-side-navigation__label">Sign out</span>
                    </button>
                  </li>
                </ul>
              </div>

            </div>
          </div>
        </div>
      </nav>

      <div v-if="!navCollapsed" class="nav-overlay" @click="closeNav" />

      <main class="l-main">
        <router-view v-slot="{ Component }" :key="project.selectedProjectId ?? 'no-project'">
          <component :is="Component" :project-id="project.selectedProjectId ?? null" />
        </router-view>
      </main>
    </div>

    <div v-if="showCreateProject" class="modal" @click.self="showCreateProject = false">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="showCreateProject = false">✕</button>
        <h3>New project</h3>
        <div class="form-group">
          <label for="new-proj-name">Name</label>
          <input id="new-proj-name" v-model="newProjectName" type="text" placeholder="Project name" />
        </div>
        <div class="form-group">
          <label>Description</label>
          <input v-model="newProjectDesc" type="text" placeholder="Optional description" />
        </div>
        <div class="form-group">
          <label>Group</label>
          <select v-model="newProjectGroupId" :disabled="availableGroups.length === 0">
            <option v-if="availableGroups.length === 0" value="" disabled>You are not a member of any group</option>
            <option v-for="g in availableGroups" :key="g.id" :value="g.id">{{ g.name }}</option>
          </select>
        </div>
        <p v-if="createProjectError" class="auth-error">{{ createProjectError }}</p>
        <button
          class="p-button--positive"
          type="button"
          :disabled="!newProjectName || !newProjectGroupId || creatingProject"
          @click="submitCreateProject"
        >Create</button>
      </div>
    </div>
  </template>

  <template v-else>
    <router-view />
  </template>
</template>

<script setup>
import { ref, computed, onMounted, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useAuthStore }    from './stores/auth.js';
import { useThemeStore }   from './stores/theme.js';
import { useProjectStore } from './stores/project.js';

const auth    = useAuthStore();
const theme   = useThemeStore();
const project = useProjectStore();
const route   = useRoute();
const router  = useRouter();

const navCollapsed         = ref(true);
const projectDropdownOpen  = ref(false);
const projectSearch        = ref('');
const showCreateProject    = ref(false);
const newProjectName       = ref('');
const newProjectDesc       = ref('');
const newProjectGroupId    = ref('');
const createProjectError   = ref('');
const creatingProject      = ref(false);
const availableGroups      = ref([]);

const isAuthRoute = computed(() =>
  route.path === '/login' || route.path === '/register'
);

const filteredProjects = computed(() => {
  const q = projectSearch.value.trim().toLowerCase();
  if (!q) return project.projects;
  return project.projects.filter(p =>
    p.name.toLowerCase().includes(q) ||
    (p.description ?? '').toLowerCase().includes(q) ||
    (p.group_name  ?? '').toLowerCase().includes(q)
  );
});

function toggleProjectDropdown() {
  projectDropdownOpen.value = !projectDropdownOpen.value;
  if (projectDropdownOpen.value) projectSearch.value = '';
}

function chooseProject(p) {
  project.selectProject(p);
  projectDropdownOpen.value = false;
}

function closeNav(e) {
  navCollapsed.value = true;
  if (e?.currentTarget) e.currentTarget.blur();
  else (document.activeElement)?.blur();
}

function closeNavMobile() {
  if (window.innerWidth < 620) {
    navCollapsed.value = true;
    document.activeElement?.blur();
  }
}

async function handleLogout() {
  await auth.logout();
  router.push('/login');
}

async function openCreateProject() {
  projectDropdownOpen.value = false;
  newProjectName.value  = '';
  newProjectDesc.value  = '';
  newProjectGroupId.value = '';
  createProjectError.value = '';
  availableGroups.value = await project.fetchGroups();
  if (availableGroups.value.length === 1) newProjectGroupId.value = availableGroups.value[0].id;
  showCreateProject.value = true;
}

async function submitCreateProject() {
  if (!newProjectName.value || !newProjectGroupId.value) return;
  creatingProject.value    = true;
  createProjectError.value = '';
  try {
    const created = await project.createProject({
      name:        newProjectName.value,
      description: newProjectDesc.value,
      group_id:    newProjectGroupId.value,
    });
    showCreateProject.value = false;
    project.selectProjectById(created.id);
  } catch (e) {
    createProjectError.value = e.message;
  } finally {
    creatingProject.value = false;
  }
}

onMounted(async () => {
  auth.fetchConfig();
  if (!auth.isLoggedIn) {
    const user = await auth.fetchMe();
    if (!user && !isAuthRoute.value) {
      router.push('/login');
      return;
    }
    if (user?.last_project_id) project.selectProjectById(user.last_project_id);
  }
  if (auth.isLoggedIn) {
    await project.fetchProjects();
  }
});

watch(() => auth.isLoggedIn, async (loggedIn) => {
  if (loggedIn) {
    await project.fetchProjects();
    if (auth.user?.last_project_id) project.selectProjectById(auth.user.last_project_id);
  }
});
</script>

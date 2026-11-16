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
              <img src="/canonical-logo.png" alt="Canonical" class="p-panel__logo-image" height="40" />
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
                <img src="/canonical-logo.png" alt="Canonical" class="p-panel__logo-image" height="40" />
                <span class="p-heading--4 is-fading-when-collapsed">Harvest</span>
              </a>
              <div class="p-panel__controls u-hide--medium u-hide--large">
                <button
                  class="p-button--base has-icon u-no-margin is-dense"
                  type="button"
                  aria-label="Close navigation"
                  @click="closeNav"
                >
                  <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                </button>
              </div>
            </div>

            <div class="p-panel__content">
              <ModeNavigation :mode="space.mode" @navigate="closeNavMobile">
                <div v-if="usesProjects(space.mode)" class="project-selector-section">
                  <span class="project-selector__label is-fading-when-collapsed">Project</span>
                  <button
                    class="project-selector__toggle"
                    type="button"
                    :aria-expanded="projectDropdownOpen"
                    @click="toggleProjectDropdown"
                  >
                    <svg class="project-selector__folder-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
                    <span class="project-selector__name is-fading-when-collapsed">{{ project.selectedProject?.name ?? 'Select project' }}</span>
                    <svg class="project-selector__chevron is-fading-when-collapsed" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="4 6 8 10 12 6"/></svg>
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
              </ModeNavigation>

              <div v-if="usesProjects(space.mode) && projectCostLabel" class="nav-cost-block is-fading-when-collapsed">
                <div class="nav-cost-block__row">
                  <span class="nav-cost-block__label">Chat spend</span>
                  <span class="nav-cost-block__value">{{ chatCostLabel }}</span>
                </div>
                <div class="nav-cost-block__row">
                  <span class="nav-cost-block__label">Design spend</span>
                  <span class="nav-cost-block__value">{{ designCostLabel }}</span>
                </div>
                <div class="nav-cost-block__row">
                  <span class="nav-cost-block__label">Deploy spend</span>
                  <span class="nav-cost-block__value">{{ deployCostLabel }}</span>
                </div>
                <div class="nav-cost-block__row nav-cost-block__row--total">
                  <span class="nav-cost-block__label nav-cost-block__label--total">Total project spend</span>
                  <span class="nav-cost-block__value nav-cost-block__value--total">{{ projectCostLabel }}</span>
                </div>
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
                    <router-link to="/settings" active-class="is-active" class="p-side-navigation__link" @click="closeNavMobile">
                      <svg class="p-side-navigation__icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
                      <span class="p-side-navigation__label">Settings</span>
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
        <router-view v-slot="{ Component }" :key="routeViewKey">
          <component :is="Component" :project-id="space.projectId" :personal="space.isPersonal" />
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
          class="p-button--positive is-dense"
          type="button"
          :disabled="!newProjectName || !newProjectGroupId || creatingProject"
          @click="submitCreateProject"
        >Create</button>
      </div>
    </div>

    <div v-if="auth.apiKeyRequired && !auth.apiKeyDismissed && route.path !== '/settings'" class="modal">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="auth.apiKeyDismissed = true" style="display:none"></button>
        <h3>API key required</h3>
        <p class="modal-lede">
          <template v-if="auth.apiKeyProviderName">An API key is required for <strong>{{ auth.apiKeyProviderName }}</strong>.</template>
          <template v-else>An API key is required.</template>
          Please go to <strong>Settings</strong> to add your API key.
        </p>
        <div class="modal-actions">
          <button class="p-button--positive is-dense" type="button" @click="goToSettings">Go to Settings</button>
        </div>
      </div>
    </div>
  </template>

  <template v-else>
    <router-view />
  </template>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useAuthStore }    from './stores/auth.js';
import { useThemeStore }   from './stores/theme.js';
import { useProjectStore } from './stores/project.js';
import { useLlmStore }    from './stores/llm.js';
import { useSpaceStore }  from './stores/space.js';
import ModeNavigation from './components/nav/ModeNavigation.vue';
import { usesProjects } from './lib/modes.js';
import { fetchLlmProviders, fetchLlmUserKeys, fetchProjectCost } from './lib/api.js';

const auth    = useAuthStore();
const theme   = useThemeStore();
const project = useProjectStore();
const llm     = useLlmStore();
const space   = useSpaceStore();
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
const projectCostMicrousd  = ref(0);
const chatCostMicrousd     = ref(0);
const designCostMicrousd   = ref(0);
const deployCostMicrousd   = ref(0);
let   costRefreshTimer     = null;

function formatMicrousd(microusd) {
  if (microusd <= 0) return '$0.00';
  const dollars = microusd / 1_000_000;
  if (dollars < 0.01) return `$${dollars.toFixed(6)}`;
  if (dollars < 1) return `$${dollars.toFixed(4)}`;
  return `$${dollars.toFixed(2)}`;
}

const chatCostLabel   = computed(() => formatMicrousd(chatCostMicrousd.value));
const designCostLabel = computed(() => formatMicrousd(designCostMicrousd.value));
const deployCostLabel = computed(() => formatMicrousd(deployCostMicrousd.value));

const projectCostLabel = computed(() => {
  if (projectCostMicrousd.value <= 0 && chatCostMicrousd.value <= 0 && designCostMicrousd.value <= 0 && deployCostMicrousd.value <= 0) return null;
  return formatMicrousd(projectCostMicrousd.value);
});

async function refreshProjectCost() {
  if (!project.selectedProjectId) {
    projectCostMicrousd.value = 0;
    chatCostMicrousd.value = 0;
    designCostMicrousd.value = 0;
    deployCostMicrousd.value = 0;
    return;
  }
  try {
    const data = await fetchProjectCost(project.selectedProjectId);
    projectCostMicrousd.value = data?.total?.total_cost_microusd ?? 0;
    const scopes = data?.by_scope ?? {};
    chatCostMicrousd.value   = scopes.chat?.total_cost_microusd ?? 0;
    designCostMicrousd.value = (scopes.design?.total_cost_microusd ?? 0) + (scopes.proposal?.total_cost_microusd ?? 0) * (scopes.proposal ? 0 : 0);
    designCostMicrousd.value = scopes.design?.total_cost_microusd ?? 0;
    deployCostMicrousd.value = (scopes.provision?.total_cost_microusd ?? 0) + (scopes.proposal?.total_cost_microusd ?? 0);
  } catch {
    projectCostMicrousd.value = 0;
    chatCostMicrousd.value = 0;
    designCostMicrousd.value = 0;
    deployCostMicrousd.value = 0;
  }
}

const isAuthRoute = computed(() =>
  route.path === '/login' || route.path === '/register' || route.path.startsWith('/authenticate/')
);

const routeViewKey = computed(() =>
  route.meta.stableAcrossProjects ? route.path : (space.projectId ?? space.mode)
);

watch(() => route.meta.mode, (mode) => space.enter(mode), { immediate: true });

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
  try {
    auth.fetchConfig();
    if (!auth.isLoggedIn) {
      const user = await auth.fetchMe();
      if (!user && !isAuthRoute.value) {
        router.push('/login');
        return;
      }
    }
    if (auth.isLoggedIn) {
      const next = sessionStorage.getItem('tui_auth_next');
      if (next) {
        sessionStorage.removeItem('tui_auth_next');
        router.push(next);
        return;
      }
      if (auth.user?.last_project_id) project.selectProjectById(auth.user.last_project_id);
      await project.fetchProjects();
      await checkApiKeys();
      refreshProjectCost();
      costRefreshTimer = setInterval(refreshProjectCost, 30_000);
    }
  } finally {
    project.markReady();
  }
});

async function checkApiKeys() {
  try {
    const [providersRes, keysRes] = await Promise.all([
      fetchLlmProviders(),
      fetchLlmUserKeys(),
    ]);
    const visibleProviders = providersRes.providers ?? [];
    const userKeyProviders = keysRes.providers ?? [];
    if (userKeyProviders.length > 0 && visibleProviders.length === 0) {
      if (userKeyProviders.length === 1) {
        auth.apiKeyProviderName = userKeyProviders[0]?.name || userKeyProviders[0]?.kind || 'the LLM provider';
      } else {
        auth.apiKeyProviderName = '';
      }
      if (!auth.apiKeyRequired) auth.apiKeyDismissed = false;
      auth.apiKeyRequired = true;
    } else {
      auth.apiKeyRequired = false;
    }
  } catch {}
}

function goToSettings() {
  auth.apiKeyDismissed = true;
  router.push('/settings');
}

watch(() => auth.isLoggedIn, async (loggedIn) => {
  if (loggedIn) {
    await project.fetchProjects();
    if (auth.user?.last_project_id) project.selectProjectById(auth.user.last_project_id);
  }
});

watch(() => project.selectedProjectId, () => {
  refreshProjectCost();
});

watch(() => llm.userKeyProviders, async () => {
  if (auth.isLoggedIn) {
    await checkApiKeys();
  }
}, { deep: true });

onUnmounted(() => {
  if (costRefreshTimer) clearInterval(costRefreshTimer);
});
</script>

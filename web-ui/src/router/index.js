import { createRouter, createWebHashHistory } from 'vue-router';
import { useAuthStore } from '../stores/auth.js';

const routes = [
  { path: '/login',          component: () => import('../views/LoginView.vue'),         meta: { public: true } },
  { path: '/register',       component: () => import('../views/RegisterView.vue'),       meta: { public: true } },
  { path: '/',               component: () => import('../views/ChatView.vue'),           meta: { requiresProject: true } },
  { path: '/tasks',          component: () => import('../views/TasksView.vue'),          meta: { requiresProject: true } },
  { path: '/agents',         component: () => import('../views/AgentsView.vue'),         meta: { requiresProject: true } },
  { path: '/agents/:agentId/console', component: () => import('../views/AgentConsoleView.vue'), meta: { requiresProject: true } },
  { path: '/memories',       component: () => import('../views/MemoriesView.vue'),       meta: { requiresProject: true } },
  { path: '/skills',         component: () => import('../views/SkillsView.vue'),         meta: { requiresProject: true } },
  { path: '/docs',           component: () => import('../views/DocumentationView.vue'),  meta: { requiresProject: true, feature: 'docs' } },
  { path: '/repositories',   component: () => import('../views/RepositoriesView.vue'),   meta: { requiresProject: true } },
  { path: '/admin',          component: () => import('../views/AdminView.vue'),          meta: { requiresAuth: true } },
  { path: '/:pathMatch(.*)*', redirect: '/' },
];

const router = createRouter({
  history: createWebHashHistory(),
  routes,
});

router.beforeEach(async (to, from) => {
  if (to.meta.public) return true;

  const auth = useAuthStore();
  if (!auth.isLoggedIn) {
    const user = await auth.fetchMe();
    if (!user) return '/login';
  }

  if (to.meta.feature && !auth.features[to.meta.feature]) return '/';

  if (from.matched.length === 0 && to.path !== '/') return '/';

  return true;
});

export default router;

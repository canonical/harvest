import { createRouter, createWebHashHistory } from 'vue-router';
import { useAuthStore } from '../stores/auth.js';

const ChatWorkspaceView = () => import('../views/ChatWorkspaceView.vue');
const ArtifactsView = () => import('../views/ArtifactsView.vue');
const SkillsView = () => import('../views/SkillsView.vue');
const AgentConsoleView = () => import('../views/AgentConsoleView.vue');

const routes = [
  { path: '/login',          component: () => import('../views/LoginView.vue'),         meta: { public: true } },
  { path: '/register',       component: () => import('../views/RegisterView.vue'),       meta: { public: true } },
  { path: '/preview/:id',    component: () => import('../views/PreviewView.vue'), props: true, meta: { public: true, bare: true } },
  { path: '/authenticate/:uuid', component: () => import('../views/AuthenticateView.vue'), meta: { public: true } },
  { path: '/',               redirect: '/chat' },

  { path: '/chat',           component: ChatWorkspaceView, meta: { mode: 'chat', stableAcrossProjects: true, requiresApiKey: true } },
  { path: '/chat/artifacts', component: ArtifactsView,     meta: { mode: 'chat', directEntry: true } },
  { path: '/chat/skills',    component: SkillsView,        meta: { mode: 'chat', directEntry: true } },

  { path: '/code',           component: () => import('../views/CodeSessionsView.vue'), meta: { mode: 'code', directEntry: true } },
  { path: '/code/projects/:id', component: () => import('../views/CodeProjectView.vue'), props: true, meta: { mode: 'code', directEntry: true } },
  { path: '/code/sessions/:id', component: () => import('../views/CodeSessionView.vue'), props: true, meta: { mode: 'code', directEntry: true } },
  { path: '/code/repositories', component: () => import('../views/RepositoriesView.vue'), meta: { mode: 'code', directEntry: true } },

  { path: '/deploy',                   redirect: '/deploy/chat' },
  { path: '/deploy/chat',              component: ChatWorkspaceView, meta: { mode: 'deploy', requiresProject: true, stableAcrossProjects: true, requiresApiKey: true } },
  { path: '/deploy/design',            component: () => import('../views/DesignView.vue'), meta: { mode: 'deploy', requiresProject: true, directEntry: true, requiresApiKey: true } },
  { path: '/deploy/deployments',       component: () => import('../views/DeployView.vue'), meta: { mode: 'deploy', requiresProject: true, directEntry: true, requiresApiKey: true } },
  { path: '/deploy/agents',            component: () => import('../views/AgentsView.vue'), meta: { mode: 'deploy', requiresProject: true, directEntry: true } },
  { path: '/deploy/agents/:agentId/console', component: AgentConsoleView, meta: { mode: 'deploy', requiresProject: true } },
  { path: '/deploy/artifacts',         component: ArtifactsView, meta: { mode: 'deploy', requiresProject: true, directEntry: true } },
  { path: '/deploy/skills',            component: SkillsView,    meta: { mode: 'deploy', requiresProject: true, directEntry: true } },
  { path: '/deploy/settings',          component: () => import('../views/ProjectSettingsView.vue'), meta: { mode: 'deploy', requiresProject: true, directEntry: true } },
  { path: '/deploy/templates',         component: () => import('../views/ProductTemplatesView.vue'), meta: { mode: 'deploy', directEntry: true } },

  { path: '/artifacts/:id',  component: ArtifactsView, meta: { directEntry: true } },
  { path: '/docs',           component: () => import('../views/DocumentationView.vue'),  meta: { feature: 'docs' } },
  { path: '/admin',          component: () => import('../views/AdminView.vue'),          meta: { requiresAuth: true } },
  { path: '/settings',       component: () => import('../views/SettingsView.vue'),       meta: { requiresAuth: true, stableAcrossProjects: true } },

  { path: '/design',            redirect: '/deploy/design' },
  { path: '/deployments',       redirect: '/deploy/deployments' },
  { path: '/deployments/:id',   redirect: '/deploy/deployments' },
  { path: '/agents',            redirect: '/deploy/agents' },
  { path: '/agents/:agentId/console', redirect: (to) => `/deploy/agents/${to.params.agentId}/console` },
  { path: '/artifacts',         redirect: '/deploy/artifacts' },
  { path: '/skills',            redirect: '/deploy/skills' },
  { path: '/repositories',      redirect: '/code/repositories' },
  { path: '/product-templates', redirect: '/deploy/templates' },
  { path: '/:pathMatch(.*)*',   redirect: '/chat' },
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

  if (to.meta.feature && !auth.features[to.meta.feature]) return '/chat';

  if (to.meta.requiresApiKey && auth.apiKeyRequired) return '/settings';

  if (!to.meta.directEntry && from.matched.length === 0 && to.path !== '/chat') return '/chat';

  return true;
});

export default router;

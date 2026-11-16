export const MODE = Object.freeze({
  CHAT: 'chat',
  CODE: 'code',
  DEPLOY: 'deploy',
});

export const MODES = Object.freeze([
  { id: MODE.CHAT, label: 'Chat', home: '/chat', icon: 'chat' },
  { id: MODE.CODE, label: 'Code', home: '/code', icon: 'code' },
  { id: MODE.DEPLOY, label: 'Deploy', home: '/deploy/chat', icon: 'deploy' },
]);

const PAGES = Object.freeze({
  [MODE.CHAT]: [
    { to: '/chat', label: 'Chat', icon: 'chat', exact: true },
    { to: '/chat/artifacts', label: 'Artifacts', icon: 'artifacts' },
    { to: '/chat/skills', label: 'Skills', icon: 'skills' },
  ],
  [MODE.CODE]: [
    { to: '/code', label: 'Repositories', icon: 'code', exact: true },
  ],
  [MODE.DEPLOY]: [
    { to: '/deploy/chat', label: 'Chat', icon: 'chat' },
    { to: '/deploy/design', label: 'Design', icon: 'design' },
    { to: '/deploy/deployments', label: 'Deployments', icon: 'deploy' },
    { to: '/deploy/artifacts', label: 'Artifacts', icon: 'artifacts' },
    { to: '/deploy/agents', label: 'Agents', icon: 'agents' },
    { to: '/deploy/skills', label: 'Skills', icon: 'skills' },
    { to: '/deploy/templates', label: 'Product templates', icon: 'templates' },
    { to: '/deploy/settings', label: 'Project settings', icon: 'members' },
  ],
});

export function pagesFor(mode) {
  return PAGES[mode] ?? [];
}

export function usesProjects(mode) {
  return mode === MODE.DEPLOY;
}

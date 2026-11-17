import { describe, it, expect } from 'vitest';
import { mount } from '@vue/test-utils';
import { createRouter, createMemoryHistory } from 'vue-router';
import ModeNavigation from '../../src/components/nav/ModeNavigation.vue';
import { MODE, pagesFor, usesProjects } from '../../src/lib/modes.js';

const router = createRouter({
  history: createMemoryHistory(),
  routes: [{ path: '/:any(.*)*', component: { template: '<div />' } }],
});

async function navigation(mode, slot = '') {
  router.push('/chat');
  await router.isReady();
  return mount(ModeNavigation, { props: { mode }, slots: { default: slot }, global: { plugins: [router] } });
}

describe('ModeNavigation', () => {
  it('offers the three modes and marks the current one', async () => {
    const w = await navigation(MODE.CODE);
    const tabs = w.findAll('.mode-switch__item');
    expect(tabs.map(t => t.text())).toEqual(['Chat', 'Code', 'Deploy']);
    expect(tabs[1].attributes('aria-selected')).toBe('true');
  });

  it('lists only the pages of the current mode', async () => {
    const chat = await navigation(MODE.CHAT);
    expect(chat.findAll('.p-side-navigation__label').map(l => l.text())).toEqual(['Chat', 'Artifacts', 'Skills', 'Knowledge']);
    const deploy = await navigation(MODE.DEPLOY);
    expect(deploy.findAll('.p-side-navigation__label').map(l => l.text())).toContain('Agents');
  });

  it('places the project selector between the mode switch and the pages', async () => {
    const w = await navigation(MODE.DEPLOY, '<div class="selector">project</div>');
    const html = w.html();
    expect(html.indexOf('mode-switch')).toBeLessThan(html.indexOf('selector'));
    expect(html.indexOf('selector')).toBeLessThan(html.indexOf('sidenav-top-container'));
  });
});

describe('modes', () => {
  it('keeps agents out of chat and code', () => {
    for (const mode of [MODE.CHAT, MODE.CODE]) {
      expect(pagesFor(mode).some(p => p.icon === 'agents')).toBe(false);
    }
  });

  it('offers knowledge in chat and deploy but not in code', () => {
    const offersKnowledge = (mode) => pagesFor(mode).some((page) => page.label === 'Knowledge');
    expect(offersKnowledge(MODE.CHAT)).toBe(true);
    expect(offersKnowledge(MODE.DEPLOY)).toBe(true);
    expect(offersKnowledge(MODE.CODE)).toBe(false);
  });

  it('uses projects only in deploy mode', () => {
    expect(usesProjects(MODE.DEPLOY)).toBe(true);
    expect(usesProjects(MODE.CHAT)).toBe(false);
    expect(usesProjects(MODE.CODE)).toBe(false);
  });
});

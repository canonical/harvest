import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';

async function freshRouter() {
  vi.resetModules();
  const mod = await import('../../src/router/index.js');
  return mod.default;
}

describe('router beforeEach fresh-load guard', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    global.fetch = vi.fn(() => Promise.resolve({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ id: 'u1', email: 'a@x.com', name: 'Alice', role: 'regular' }),
    }));
  });

  it('allows a direct deep link into a directEntry route on a fresh load', async () => {
    const router = await freshRouter();
    await router.push('/artifacts/abc-123');
    await router.isReady();
    expect(router.currentRoute.value.path).toBe('/artifacts/abc-123');
  });

  it('sends a fresh load of a page that needs context back to the chat home', async () => {
    const router = await freshRouter();
    await router.push('/deploy/chat');
    await router.isReady();
    expect(router.currentRoute.value.path).toBe('/chat');
  });

  it('opens chat mode from the root', async () => {
    const router = await freshRouter();
    await router.push('/');
    await router.isReady();
    expect(router.currentRoute.value.path).toBe('/chat');
  });

  it('moves the old deploy pages under deploy mode', async () => {
    for (const [from, to] of [
      ['/deployments/abc-123', '/deploy/deployments'],
      ['/deployments', '/deploy/deployments'],
      ['/agents', '/deploy/agents'],
      ['/skills', '/deploy/skills'],
      ['/repositories', '/chat/knowledge'],
    ]) {
      const router = await freshRouter();
      await router.push(from);
      await router.isReady();
      expect(router.currentRoute.value.path).toBe(to);
    }
  });

  it('tags every page with the mode it belongs to', async () => {
    const router = await freshRouter();
    const modes = Object.fromEntries(router.getRoutes().map(r => [r.path, r.meta.mode]));
    expect(modes['/chat']).toBe('chat');
    expect(modes['/chat/skills']).toBe('chat');
    expect(modes['/code']).toBe('code');
    expect(modes['/deploy/agents']).toBe('deploy');
  });
});

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

  it('still redirects a non-directEntry route to / on a fresh load', async () => {
    const router = await freshRouter();
    await router.push('/tasks');
    await router.isReady();
    expect(router.currentRoute.value.path).toBe('/');
  });
});

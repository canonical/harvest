import { describe, it, expect, vi, beforeEach } from 'vitest';
import { useAuthStore } from '../../src/stores/auth.js';

function mockFetch(responses) {
  let call = 0;
  global.fetch = vi.fn(() => {
    const r = responses[call++] ?? responses.at(-1);
    return Promise.resolve({
      ok: r.ok ?? true,
      status: r.status ?? 200,
      json: () => Promise.resolve(r.body ?? {}),
    });
  });
}

describe('useAuthStore', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('starts with null user', () => {
    const store = useAuthStore();
    expect(store.user).toBeNull();
    expect(store.isAdmin).toBe(false);
    expect(store.isLoggedIn).toBe(false);
  });

  it('fetchMe sets user on success', async () => {
    mockFetch([{ body: { id: '1', email: 'a@b.com', name: 'Alice', role: 'admin' } }]);
    const store = useAuthStore();
    await store.fetchMe();
    expect(store.user).toEqual({ id: '1', email: 'a@b.com', name: 'Alice', role: 'admin' });
    expect(store.isLoggedIn).toBe(true);
    expect(store.isAdmin).toBe(true);
  });

  it('fetchMe returns null on 401', async () => {
    mockFetch([{ ok: false, status: 401, body: {} }]);
    const store = useAuthStore();
    const result = await store.fetchMe();
    expect(result).toBeNull();
    expect(store.user).toBeNull();
  });

  it('login POSTs credentials', async () => {
    mockFetch([{ body: { ok: true } }]);
    const store = useAuthStore();
    await store.login('a@b.com', 'pw123');
    expect(global.fetch).toHaveBeenCalledWith('/auth/login', expect.objectContaining({
      method: 'POST',
      body: JSON.stringify({ email: 'a@b.com', password: 'pw123' }),
    }));
  });

  it('login throws on error response', async () => {
    mockFetch([{ ok: false, status: 401, body: { error: 'invalid credentials' } }]);
    const store = useAuthStore();
    await expect(store.login('bad@b.com', 'wrong')).rejects.toThrow();
  });

  it('logout clears user', async () => {
    mockFetch([{ body: { ok: true } }]);
    const store = useAuthStore();
    store.user = { id: '1', name: 'Alice', email: 'a@b.com', role: 'regular' };
    await store.logout();
    expect(store.user).toBeNull();
  });

  it('isAdmin false for regular user', async () => {
    mockFetch([{ body: { id: '2', email: 'b@b.com', name: 'Bob', role: 'regular' } }]);
    const store = useAuthStore();
    await store.fetchMe();
    expect(store.isAdmin).toBe(false);
  });
});

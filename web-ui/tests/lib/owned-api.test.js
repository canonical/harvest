import { describe, it, expect, vi, afterEach } from 'vitest';
import { artifactCollection, skillCollection } from '../../src/lib/owned-api.js';

afterEach(() => vi.unstubAllGlobals());

function stubFetch() {
  const calls = [];
  vi.stubGlobal('fetch', vi.fn((url, options = {}) => {
    calls.push({ url, method: options.method ?? 'GET' });
    return Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve([]) });
  }));
  return calls;
}

describe('owned collections', () => {
  it('reads personal artifacts and skills from the current user', async () => {
    const calls = stubFetch();
    await artifactCollection(null).list();
    await skillCollection(null).list();
    await skillCollection(null).create({ name: 'notes' });
    expect(calls.map(c => `${c.method} ${c.url}`)).toEqual([
      'GET /me/artifacts', 'GET /me/skills', 'POST /me/skills',
    ]);
  });

  it('reads project artifacts and skills from the project', async () => {
    const calls = stubFetch();
    await artifactCollection('p1').list();
    await skillCollection('p1').list();
    expect(calls.every(c => c.url.startsWith('/projects/p1/'))).toBe(true);
  });
});

import { describe, it, expect, vi, afterEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import GitAccessSection from '../../src/components/settings/GitAccessSection.vue';

const PLATFORMS = [
  { id: 'github', label: 'GitHub', default_host: 'github.com', needs_username: false },
  { id: 'gerrit', label: 'OpenDev (Gerrit)', default_host: 'review.opendev.org', needs_username: true },
];
const KEY = { public_key: 'ssh-ed25519 AAAA harvest', fingerprint: 'SHA256:abc', created_at: '2026-11-16T00:00:00Z' };

function respond(body, status = 200) {
  return Promise.resolve({ ok: status < 400, status, json: () => Promise.resolve(body) });
}

function stubFetch(handler) {
  const calls = [];
  vi.stubGlobal('fetch', vi.fn((url, options = {}) => {
    calls.push({ url, method: options.method ?? 'GET', body: options.body ? JSON.parse(options.body) : null });
    return handler(url, options);
  }));
  return calls;
}

afterEach(() => vi.unstubAllGlobals());

describe('GitAccessSection', () => {
  it('generates an SSH key and shows its public half', async () => {
    const calls = stubFetch((url, options) => (
      url === '/me/ssh-key' && options.method === 'PUT'
        ? respond(KEY)
        : respond({ ssh_key: null, accounts: [], platforms: PLATFORMS })
    ));
    const w = mount(GitAccessSection);
    await flushPromises();
    await w.findAll('button').find(b => b.text() === 'Generate a key').trigger('click');
    await flushPromises();
    expect(calls.at(-1)).toMatchObject({ url: '/me/ssh-key', method: 'PUT' });
    expect(w.find('.git-access__public').text()).toBe(KEY.public_key);
  });

  it('asks for a username only on platforms that need one', async () => {
    stubFetch(() => respond({ ssh_key: KEY, accounts: [], platforms: PLATFORMS }));
    const w = mount(GitAccessSection);
    await flushPromises();
    await w.findAll('button').find(b => b.text() === 'Add account').trigger('click');
    expect(w.find('#account-host').element.value).toBe('github.com');
    expect(w.find('#account-username').exists()).toBe(false);
    await w.find('#account-platform').setValue('gerrit');
    expect(w.find('#account-host').element.value).toBe('review.opendev.org');
    expect(w.find('#account-username').exists()).toBe(true);
  });

  it('saves a platform account and lists it', async () => {
    const saved = { platform: 'github', host: 'github.com', username: '', verified_at: null };
    const calls = stubFetch((url, options) => (
      url === '/me/accounts' && options.method === 'PUT'
        ? respond({ accounts: [saved] })
        : respond({ ssh_key: null, accounts: [], platforms: PLATFORMS })
    ));
    const w = mount(GitAccessSection);
    await flushPromises();
    await w.findAll('button').find(b => b.text() === 'Add account').trigger('click');
    await w.find('#account-secret').setValue('ghp_token');
    await w.findAll('.modal button').find(b => b.text() === 'Save').trigger('click');
    await flushPromises();
    expect(calls.at(-1).body).toEqual({ platform: 'github', host: 'github.com', username: '', secret: 'ghp_token' });
    expect(w.find('.provider-card__name').text()).toBe('GitHub');
  });
});

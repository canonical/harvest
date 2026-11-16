import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import AdminView from '../../src/views/AdminView.vue';

const USERS = [
  { id: 'u1', name: 'Alice Admin', email: 'alice@example.com', role: 'admin', provider: 'local' },
  { id: 'u2', name: 'Bob User', email: 'bob@example.com', role: 'regular', provider: 'google' },
];

const MOVES = [
  { id: 'c1', title: 'How does search rank?', project_name: 'Test', author_email: 'alice@example.com' },
  { id: 'c2', title: 'Where is the parser?', project_name: 'Test', author_email: 'bob@example.com' },
];

function mockFetch() {
  const calls = [];
  global.fetch = vi.fn((url, options = {}) => {
    calls.push({ url, method: options.method ?? 'GET', body: options.body ? JSON.parse(options.body) : null });
    const respond = (body) => Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve(body) });
    if (url.includes('/admin/users')) return respond(USERS);
    if (url.includes('/admin/conversation-moves') && options.method === 'POST') return respond({ moved: 1 });
    if (url.includes('/admin/conversation-moves')) return respond(MOVES);
    return respond({});
  });
  return calls;
}

function mountAdmin() {
  return mount(AdminView, { global: { plugins: [createPinia()] } });
}

describe('AdminView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('lists users with their role and counts them', async () => {
    mockFetch();
    const w = mountAdmin();
    await flushPromises();
    expect(w.text()).toContain('Alice Admin');
    expect(w.text()).toContain('bob@example.com');
    expect(w.text()).toMatch(/2\s*users/);
    expect(w.text()).not.toMatch(/group/i);
  });

  it('changes a user role', async () => {
    const calls = mockFetch();
    const w = mountAdmin();
    await flushPromises();
    await w.findAll('button').filter(b => b.text() === 'Edit')[1].trigger('click');
    await w.find('#edit-user-role').setValue('admin');
    await w.findAll('button').find(b => b.text() === 'Save').trigger('click');
    await flushPromises();
    expect(calls.find(c => c.method === 'PUT')).toMatchObject({ url: '/admin/users/u2/role', body: { role: 'admin' } });
  });

  it('moves the selected code-only chats to chat mode', async () => {
    const calls = mockFetch();
    const w = mountAdmin();
    await flushPromises();
    await w.find('[data-testid="tab-moves"]').trigger('click');
    await flushPromises();
    expect(w.text()).toContain('How does search rank?');
    await w.findAll('input[type="checkbox"]')[2].setValue(false);
    await w.findAll('button').find(b => b.text().startsWith('Move 1 chat')).trigger('click');
    await flushPromises();
    expect(calls.find(c => c.method === 'POST')).toMatchObject({ url: '/admin/conversation-moves', body: { ids: ['c1'] } });
    expect(w.text()).toContain('Moved 1 chat to Chat mode.');
  });
});

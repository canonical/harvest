import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import AdminView from '../../src/views/AdminView.vue';

const USERS = [
  { id: 'u1', name: 'Alice Admin', email: 'alice@example.com', role: 'admin',   group_ids: ['g1'], provider: 'local' },
  { id: 'u2', name: 'Bob User',   email: 'bob@example.com',   role: 'regular', group_ids: [],     provider: 'google' },
];

const GROUPS = [
  { id: 'g1', name: 'Engineering', description: 'Dev team' },
  { id: 'g2', name: 'Design',      description: '' },
];

function mockFetch({ users = USERS, groups = GROUPS } = {}) {
  global.fetch = vi.fn((url) => {
    if (url.includes('/admin/users')) {
      return Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve(users) });
    }
    if (url.includes('/admin/groups')) {
      return Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve(groups) });
    }
    return Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve({}) });
  });
}

describe('AdminView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('shows users tab and groups tab', async () => {
    mockFetch();
    const w = mount(AdminView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    expect(w.text()).toMatch(/users/i);
    expect(w.text()).toMatch(/groups/i);
  });

  it('renders user list after load', async () => {
    mockFetch();
    const w = mount(AdminView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    expect(w.text()).toContain('Alice Admin');
    expect(w.text()).toContain('alice@example.com');
    expect(w.text()).toContain('Bob User');
  });

  it('shows stats header with user and group counts', async () => {
    mockFetch();
    const w = mount(AdminView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    expect(w.text()).toMatch(/2\s*user/i);
    expect(w.text()).toMatch(/2\s*group/i);
  });

  it('switches to groups tab and shows group list', async () => {
    mockFetch();
    const w = mount(AdminView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    const tabBtn = w.findAll('button').find(b => b.text().match(/groups/i));
    expect(tabBtn).toBeDefined();
    await tabBtn.trigger('click');
    expect(w.text()).toContain('Engineering');
    expect(w.text()).toContain('Design');
  });

  it('opens edit user modal when Edit button clicked', async () => {
    mockFetch();
    const w = mount(AdminView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    const editBtn = w.findAll('button').find(b => b.text() === 'Edit');
    expect(editBtn).toBeDefined();
    await editBtn.trigger('click');
    expect(w.find('.modal, dialog').exists()).toBe(true);
    expect(w.text()).toMatch(/role/i);
  });

  it('opens create group modal when add group button clicked', async () => {
    mockFetch();
    const w = mount(AdminView, { global: { plugins: [createPinia()] } });
    await flushPromises();
    const groupsTabBtn = w.findAll('button').find(b => b.text().match(/groups/i));
    await groupsTabBtn.trigger('click');
    const addBtn = w.findAll('button').find(b => b.text().match(/add group|create group|\+ group/i));
    expect(addBtn).toBeDefined();
    await addBtn.trigger('click');
    expect(w.find('.modal, dialog').exists()).toBe(true);
    expect(w.find('input[name="group-name"], #group-name-input').exists() || w.text().match(/group name/i)).toBeTruthy();
  });
});

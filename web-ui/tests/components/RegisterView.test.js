import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createRouter, createWebHashHistory } from 'vue-router';
import RegisterView from '../../src/views/RegisterView.vue';

function makeRouter() {
  return createRouter({
    history: createWebHashHistory(),
    routes: [
      { path: '/', component: { template: '<div>Home</div>' } },
      { path: '/login', component: { template: '<div>Login</div>' } },
      { path: '/register', component: RegisterView },
    ],
  });
}

function mockFetch(ok, body = {}) {
  global.fetch = vi.fn().mockResolvedValue({
    ok,
    status: ok ? 200 : 400,
    json: () => Promise.resolve(body),
  });
}

describe('RegisterView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('renders with .auth-page wrapper', () => {
    const w = mount(RegisterView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    expect(w.find('.auth-page').exists()).toBe(true);
  });

  it('renders with .auth-card inner card', () => {
    const w = mount(RegisterView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    expect(w.find('.auth-card').exists()).toBe(true);
  });

  it('renders email, name, password, confirm fields', async () => {
    const router = makeRouter();
    const w = mount(RegisterView, { global: { plugins: [createPinia(), router] } });
    expect(w.find('input[type="email"]').exists()).toBe(true);
    expect(w.find('input[name="name"]').exists()).toBe(true);
    expect(w.findAll('input[type="password"]')).toHaveLength(2);
  });

  it('shows error when passwords do not match', async () => {
    const router = makeRouter();
    const w = mount(RegisterView, { global: { plugins: [createPinia(), router] } });
    await w.find('input[type="email"]').setValue('a@b.com');
    await w.find('input[name="name"]').setValue('Alice');
    const pwds = w.findAll('input[type="password"]');
    await pwds[0].setValue('password1');
    await pwds[1].setValue('password2');
    await w.find('form').trigger('submit');
    await flushPromises();
    expect(w.text()).toContain('match');
  });

  it('shows error when password is too short', async () => {
    const router = makeRouter();
    const w = mount(RegisterView, { global: { plugins: [createPinia(), router] } });
    await w.find('input[type="email"]').setValue('a@b.com');
    await w.find('input[name="name"]').setValue('Alice');
    const pwds = w.findAll('input[type="password"]');
    await pwds[0].setValue('short');
    await pwds[1].setValue('short');
    await w.find('form').trigger('submit');
    await flushPromises();
    expect(w.text()).toContain('8 characters');
  });

  it('shows API error on failed registration', async () => {
    mockFetch(false, { error: 'Email already taken' });
    const router = makeRouter();
    const w = mount(RegisterView, { global: { plugins: [createPinia(), router] } });
    await w.find('input[type="email"]').setValue('a@b.com');
    await w.find('input[name="name"]').setValue('Alice');
    const pwds = w.findAll('input[type="password"]');
    await pwds[0].setValue('longpassword');
    await pwds[1].setValue('longpassword');
    await w.find('form').trigger('submit');
    await flushPromises();
    expect(w.text()).toContain('Email already taken');
  });
});

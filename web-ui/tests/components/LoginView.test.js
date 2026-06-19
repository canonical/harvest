import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createRouter, createMemoryHistory } from 'vue-router';
import { createPinia } from 'pinia';
import LoginView from '../../src/views/LoginView.vue';

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', component: { template: '<div>home</div>' } },
      { path: '/login', component: LoginView },
    ],
  });
}

function mockConfig(cfg) {
  global.fetch = vi.fn().mockResolvedValue({
    ok: true,
    json: () => Promise.resolve(cfg),
  });
}

describe('LoginView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('renders email and password inputs', () => {
    mockConfig({ local_login: true, google: false, oidc: false });
    const w = mount(LoginView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    expect(w.find('input[type="email"]').exists()).toBe(true);
    expect(w.find('input[type="password"]').exists()).toBe(true);
  });

  it('shows login form when local_login is true', async () => {
    mockConfig({ local_login: true, google: false, oidc: false });
    const w = mount(LoginView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    await flushPromises();
    expect(w.find('form').exists()).toBe(true);
  });

  it('hides login form when local_login is false', async () => {
    mockConfig({ local_login: false, google: false, oidc: true, oidc_display_name: 'SSO' });
    const w = mount(LoginView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    await flushPromises();
    expect(w.find('#local-login-section').exists()).toBe(false);
  });

  it('shows Google button when google is true', async () => {
    mockConfig({ local_login: true, google: true, oidc: false });
    const w = mount(LoginView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    await flushPromises();
    expect(w.find('#google-login-btn').exists()).toBe(true);
  });

  it('hides Google button when google is false', async () => {
    mockConfig({ local_login: true, google: false, oidc: false });
    const w = mount(LoginView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    await flushPromises();
    expect(w.find('#google-login-btn').exists()).toBe(false);
  });

  it('shows OIDC button with display name', async () => {
    mockConfig({ local_login: true, google: false, oidc: true, oidc_display_name: 'Ubuntu One' });
    const w = mount(LoginView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    await flushPromises();
    expect(w.find('#oidc-login-btn').text()).toContain('Ubuntu One');
  });

  it('shows register link when local_login is true', async () => {
    mockConfig({ local_login: true, google: false, oidc: false });
    const w = mount(LoginView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    await flushPromises();
    expect(w.find('#register-switch').exists()).toBe(true);
  });

  it('hides register link when local_login is false', async () => {
    mockConfig({ local_login: false, google: false, oidc: true });
    const w = mount(LoginView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    await flushPromises();
    expect(w.find('#register-switch').exists()).toBe(false);
  });

  it('renders with .auth-page wrapper', async () => {
    mockConfig({ local_login: true, google: false, oidc: false });
    const w = mount(LoginView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    expect(w.find('.auth-page').exists()).toBe(true);
  });

  it('renders with .auth-card inner card', async () => {
    mockConfig({ local_login: true, google: false, oidc: false });
    const w = mount(LoginView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    expect(w.find('.auth-card').exists()).toBe(true);
  });

  it('shows error message on failed login', async () => {
    global.fetch = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({ local_login: true, google: false, oidc: false }) })
      .mockResolvedValueOnce({ ok: false, status: 401, json: () => Promise.resolve({ error: 'invalid credentials' }) });

    const w = mount(LoginView, {
      global: { plugins: [createPinia(), makeRouter()] },
    });
    await flushPromises();
    await w.find('input[type="email"]').setValue('a@b.com');
    await w.find('input[type="password"]').setValue('wrong');
    await w.find('form').trigger('submit');
    await flushPromises();
    expect(w.find('.p-notification--negative, .auth-error').text()).toContain('invalid credentials');
  });
});

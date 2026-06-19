import { defineStore } from 'pinia';
import { ref, computed } from 'vue';

export const useAuthStore = defineStore('auth', () => {
  const user     = ref(null);
  const features = ref({ docs: false });

  const isLoggedIn = computed(() => user.value !== null);
  const isAdmin    = computed(() => user.value?.role === 'admin');

  async function fetchMe() {
    const res = await fetch('/auth/me');
    if (res.status === 401 || !res.ok) return null;
    const data = await res.json();
    user.value = data;
    return data;
  }

  async function login(email, password) {
    const res = await fetch('/auth/login', {
      method:  'POST',
      headers: { 'Content-Type': 'application/json' },
      body:    JSON.stringify({ email, password }),
    });
    if (!res.ok) {
      const data = await res.json().catch(() => ({}));
      throw new Error(data.error || 'Login failed');
    }
    return res.json();
  }

  async function register(email, name, password) {
    const res = await fetch('/auth/register', {
      method:  'POST',
      headers: { 'Content-Type': 'application/json' },
      body:    JSON.stringify({ email, name, password }),
    });
    if (!res.ok) {
      const data = await res.json().catch(() => ({}));
      throw new Error(data.error || 'Registration failed');
    }
    return res.json();
  }

  async function logout() {
    await fetch('/auth/logout', { method: 'POST' });
    user.value = null;
  }

  async function fetchConfig() {
    const res = await fetch('/auth/config');
    if (!res.ok) return null;
    const data = await res.json();
    if (data?.features) features.value = { ...features.value, ...data.features };
    return data;
  }

  return { user, features, isLoggedIn, isAdmin, fetchMe, login, register, logout, fetchConfig };
});

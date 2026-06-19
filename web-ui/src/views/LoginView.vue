<template>
  <div class="auth-page">
  <div class="auth-card">
    <h1>Harvest</h1>

    <section v-if="config.local_login !== false" id="local-login-section">
      <form @submit.prevent="handleLogin">
        <div class="form-group">
          <label for="email">Email</label>
          <input id="email" v-model="email" type="email" required autocomplete="email" />
        </div>
        <div class="form-group">
          <label for="password">Password</label>
          <input id="password" v-model="password" type="password" required autocomplete="current-password" />
        </div>
        <div v-if="error" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ error }}</p>
          </div>
        </div>
        <button class="p-button--positive u-no-margin--bottom" type="submit" style="width:100%" :disabled="loading">Sign in</button>
      </form>
    </section>

    <template v-if="config.google || config.oidc">
      <hr v-if="config.local_login !== false" />
      <button v-if="config.google" id="google-login-btn" class="p-button u-no-margin--bottom" style="width:100%" type="button" @click="loginWithGoogle">
        Sign in with Google
      </button>
      <button v-if="config.oidc" id="oidc-login-btn" class="p-button u-no-margin--bottom" style="width:100%" type="button" @click="loginWithOidc">
        Sign in with {{ config.oidc_display_name || 'SSO' }}
      </button>
    </template>

    <p v-if="config.local_login !== false" id="register-switch">
      No account? <router-link to="/register">Register</router-link>
    </p>
  </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useAuthStore } from '../stores/auth.js';

const router = useRouter();
const auth   = useAuthStore();

const email    = ref('');
const password = ref('');
const error    = ref('');
const loading  = ref(false);
const config   = ref({ local_login: true, google: false, oidc: false });

onMounted(async () => {
  const cfg = await auth.fetchConfig();
  if (cfg) config.value = cfg;
});

async function handleLogin() {
  error.value = '';
  loading.value = true;
  try {
    await auth.login(email.value, password.value);
    router.push('/');
  } catch (e) {
    error.value = e.message;
  } finally {
    loading.value = false;
  }
}

function loginWithGoogle() {
  window.location.href = '/auth/google';
}

function loginWithOidc() {
  window.location.href = '/auth/oidc';
}
</script>

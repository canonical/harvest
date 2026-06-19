<template>
  <div class="auth-page">
  <div class="auth-card">
    <h1>Harvest</h1>
    <form @submit.prevent="handleRegister">
      <div class="form-group">
        <label for="reg-email">Email</label>
        <input id="reg-email" v-model="email" type="email" required autocomplete="email" />
      </div>
      <div class="form-group">
        <label for="reg-name">Name</label>
        <input id="reg-name" v-model="name" type="text" name="name" required autocomplete="name" />
      </div>
      <div class="form-group">
        <label for="reg-password">Password</label>
        <input id="reg-password" v-model="password" type="password" required autocomplete="new-password" />
      </div>
      <div class="form-group">
        <label for="reg-confirm">Confirm password</label>
        <input id="reg-confirm" v-model="confirm" type="password" required autocomplete="new-password" />
      </div>
      <div v-if="error" class="p-notification--negative">
        <div class="p-notification__content">
          <p class="p-notification__message">{{ error }}</p>
        </div>
      </div>
      <button class="p-button--positive u-no-margin--bottom" type="submit" style="width:100%" :disabled="loading">Create account</button>
    </form>
    <p>Already have an account? <router-link to="/login">Sign in</router-link></p>
  </div>
  </div>
</template>

<script setup>
import { ref } from 'vue';
import { useRouter } from 'vue-router';
import { useAuthStore } from '../stores/auth.js';

const router = useRouter();
const auth   = useAuthStore();

const email    = ref('');
const name     = ref('');
const password = ref('');
const confirm  = ref('');
const error    = ref('');
const loading  = ref(false);

async function handleRegister() {
  error.value = '';
  if (password.value.length < 8) {
    error.value = 'Password must be at least 8 characters.';
    return;
  }
  if (password.value !== confirm.value) {
    error.value = 'Passwords do not match.';
    return;
  }
  loading.value = true;
  try {
    await auth.register(email.value, name.value, password.value);
    await auth.fetchMe();
    router.push('/');
  } catch (e) {
    error.value = e.message;
  } finally {
    loading.value = false;
  }
}
</script>

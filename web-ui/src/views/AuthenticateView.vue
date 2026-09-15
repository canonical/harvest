<template>
  <div class="auth-page">
    <div class="auth-card">
      <h1>Authorize Terminal</h1>

      <div v-if="loading" class="authenticate-loading">
        <p>Checking authorization request…</p>
      </div>

      <div v-else-if="status === 'pending'" class="authenticate-pending">
        <p class="authenticate-message">
          A terminal session is requesting access to your Harvest account.
        </p>
        <p class="authenticate-hint">
          If you initiated this request from the Harvest TUI, click Authorize to grant access.
        </p>
        <div class="authenticate-actions">
          <button class="p-button--positive is-dense" type="button" :disabled="authorizing" @click="authorize">
            Authorize
          </button>
          <button class="p-button--base is-dense" type="button" :disabled="authorizing" @click="deny">
            Deny
          </button>
        </div>
        <p v-if="actionError" class="p-notification--negative">
          <span class="p-notification__content">
            <span class="p-notification__message">{{ actionError }}</span>
          </span>
        </p>
      </div>

      <div v-else-if="status === 'authorized'" class="authenticate-success">
        <p>✓ Authorized. Return to your terminal to continue.</p>
      </div>

      <div v-else-if="status === 'denied'" class="authenticate-denied">
        <p>Denied. Return to your terminal for instructions.</p>
      </div>

      <div v-else-if="status === 'expired'" class="authenticate-expired">
        <p>This authorization request has expired.</p>
        <p class="authenticate-hint">Run /login again in your terminal to start a new session.</p>
      </div>

      <div v-else class="authenticate-error">
        <p>{{ error || 'Unknown error occurred.' }}</p>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useAuthStore } from '../stores/auth.js';

const route  = useRoute();
const router = useRouter();
const auth   = useAuthStore();

const loading      = ref(true);
const status       = ref('');
const error        = ref('');
const actionError  = ref('');
const authorizing  = ref(false);

onMounted(async () => {
  const uuid = route.params.uuid;
  if (!uuid) {
    error.value = 'Missing authorization ID.';
    loading.value = false;
    return;
  }

  if (!auth.isLoggedIn) {
    const user = await auth.fetchMe();
    if (!user) {
      sessionStorage.setItem('tui_auth_next', `/authenticate/${uuid}`);
      router.push('/login');
      return;
    }
  }

  await checkStatus(uuid);
});

async function checkStatus(uuid) {
  try {
    const res = await fetch(`/auth/tui/poll/${uuid}`);
    if (!res.ok) {
      error.value = 'Failed to check authorization status.';
      loading.value = false;
      return;
    }
    const data = await res.json();
    status.value = data.status;
    if (data.status === 'authorized' || data.status === 'denied') {
      setTimeout(() => router.push('/'), 3000);
    }
  } catch (e) {
    error.value = e.message;
  } finally {
    loading.value = false;
  }
}

async function authorize() {
  const uuid = route.params.uuid;
  actionError.value = '';
  authorizing.value = true;
  try {
    const res = await fetch(`/auth/tui/authorize/${uuid}`, {
      method:  'POST',
      headers: { 'Content-Type': 'application/json' },
      body:    JSON.stringify({ approved: true }),
    });
    if (!res.ok) {
      const data = await res.json().catch(() => ({}));
      actionError.value = data.error || 'Authorization failed.';
    } else {
      status.value = 'authorized';
      setTimeout(() => router.push('/'), 3000);
    }
  } catch (e) {
    actionError.value = e.message;
  } finally {
    authorizing.value = false;
  }
}

async function deny() {
  const uuid = route.params.uuid;
  actionError.value = '';
  authorizing.value = true;
  try {
    const res = await fetch(`/auth/tui/authorize/${uuid}`, {
      method:  'POST',
      headers: { 'Content-Type': 'application/json' },
      body:    JSON.stringify({ approved: false }),
    });
    if (!res.ok) {
      const data = await res.json().catch(() => ({}));
      actionError.value = data.error || 'Failed to deny.';
    } else {
      status.value = 'denied';
      setTimeout(() => router.push('/'), 3000);
    }
  } catch (e) {
    actionError.value = e.message;
  } finally {
    authorizing.value = false;
  }
}
</script>

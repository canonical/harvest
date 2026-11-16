<template>
  <section class="settings-section">
    <div class="settings-section__heading">
      <h3>Git access</h3>
      <p class="settings-section__description">
        Harvest uses your SSH key and platform tokens to read private repositories, push your branches and open change requests. They are encrypted at rest and never shared.
      </p>
    </div>

    <div v-if="loading" class="settings-loading">
      <LoadingSpinner text="Loading credentials…" />
    </div>
    <p v-else-if="loadError" class="settings-empty">{{ loadError }}</p>

    <template v-else>
      <div class="git-access__block">
        <h4 class="git-access__title">SSH key</h4>
        <div v-if="sshKey" class="git-access__key">
          <code class="git-access__public">{{ sshKey.public_key }}</code>
          <div class="git-access__meta">
            <span>{{ sshKey.fingerprint }}</span>
            <span>Created {{ formatDate(sshKey.created_at) }}</span>
          </div>
          <div class="git-access__actions">
            <button class="p-button--base is-dense" type="button" @click="copyPublicKey">{{ copied ? 'Copied' : 'Copy public key' }}</button>
            <button class="p-button--base is-dense" type="button" :disabled="busy" @click="removeSshKey">Remove</button>
          </div>
          <p class="git-access__hint">Add the public key to each git platform you use so Harvest can clone and push as you.</p>
        </div>
        <div v-else class="git-access__actions">
          <button class="p-button--positive is-dense" type="button" :disabled="busy" @click="createSshKey">Generate a key</button>
          <button class="p-button--base is-dense" type="button" :disabled="busy" @click="importing = true">Use my own key</button>
        </div>
        <div v-if="importing && !sshKey" class="form-group git-access__import">
          <label for="ssh-private-key">Private key</label>
          <textarea id="ssh-private-key" v-model="privateKeyInput" rows="5" placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" spellcheck="false"></textarea>
          <div class="git-access__actions">
            <button class="p-button--base is-dense" type="button" @click="importing = false">Cancel</button>
            <button class="p-button--positive is-dense" type="button" :disabled="!privateKeyInput.trim() || busy" @click="importKey">Save key</button>
          </div>
        </div>
      </div>

      <div class="git-access__block">
        <div class="git-access__row">
          <h4 class="git-access__title">Platform accounts</h4>
          <button class="p-button--base is-dense" type="button" @click="openAccount()">Add account</button>
        </div>
        <p v-if="!accounts.length" class="git-access__hint">No accounts yet. Add one to read private repositories over HTTPS and to open change requests.</p>
        <div v-else class="settings-providers">
          <div v-for="account in accounts" :key="account.host" class="provider-card provider-card--active">
            <div class="provider-card__body">
              <div class="provider-card__header">
                <span class="provider-card__name">{{ platformLabel(account.platform) }}</span>
                <span class="provider-card__kind">{{ account.host }}</span>
              </div>
              <div class="provider-card__meta">
                <span v-if="account.username">{{ account.username }}</span>
              </div>
            </div>
            <div class="provider-card__actions">
              <button class="provider-icon-btn" type="button" title="Update token" aria-label="Update token" @click="openAccount(account)">✎</button>
              <button class="provider-icon-btn provider-icon-btn--danger" type="button" title="Remove account" aria-label="Remove account" :disabled="busy" @click="removeAccount(account)">✕</button>
            </div>
          </div>
        </div>
      </div>
      <p v-if="actionError" class="auth-error">{{ actionError }}</p>
    </template>

    <div v-if="accountForm" class="modal" @click.self="accountForm = null">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="accountForm = null">✕</button>
        <h3>{{ accountForm.existing ? 'Update' : 'Add' }} platform account</h3>
        <div class="form-group">
          <label for="account-platform">Platform</label>
          <select id="account-platform" v-model="accountForm.platform" :disabled="accountForm.existing" @change="applyDefaultHost">
            <option v-for="p in platforms" :key="p.id" :value="p.id">{{ p.label }}</option>
          </select>
        </div>
        <div class="form-group">
          <label for="account-host">Host</label>
          <input id="account-host" v-model="accountForm.host" type="text" :disabled="accountForm.existing" placeholder="git.example.org" />
        </div>
        <div v-if="selectedPlatform?.needs_username" class="form-group">
          <label for="account-username">Username</label>
          <input id="account-username" v-model="accountForm.username" type="text" autocomplete="off" />
        </div>
        <div class="form-group">
          <label for="account-secret">{{ secretLabel }}</label>
          <input id="account-secret" v-model="accountForm.secret" type="password" autocomplete="off" />
        </div>
        <p v-if="accountError" class="auth-error">{{ accountError }}</p>
        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" @click="accountForm = null">Cancel</button>
          <button class="p-button--positive is-dense" type="button" :disabled="!canSaveAccount || busy" @click="submitAccount">Save</button>
        </div>
      </div>
    </div>
  </section>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import LoadingSpinner from '../deployment/LoadingSpinner.vue';
import {
  deleteAccount, deleteSshKey, fetchCredentials, generateSshKey, importSshKey, saveAccount,
} from '../../lib/credentials-api.js';

const COPIED_MS = 1500;

const loading = ref(true);
const loadError = ref('');
const actionError = ref('');
const busy = ref(false);
const sshKey = ref(null);
const accounts = ref([]);
const platforms = ref([]);
const importing = ref(false);
const privateKeyInput = ref('');
const copied = ref(false);
const accountForm = ref(null);
const accountError = ref('');

const selectedPlatform = computed(() => platforms.value.find(p => p.id === accountForm.value?.platform));
const secretLabel = computed(() => (accountForm.value?.platform === 'gerrit' ? 'HTTP password' : 'Access token'));
const canSaveAccount = computed(() => {
  const form = accountForm.value;
  if (!form || !form.host.trim() || !form.secret.trim()) return false;
  return !selectedPlatform.value?.needs_username || Boolean(form.username.trim());
});

function platformLabel(id) {
  return platforms.value.find(p => p.id === id)?.label ?? id;
}

function formatDate(value) {
  return value ? new Date(value).toLocaleDateString() : '';
}

async function load() {
  loading.value = true;
  try {
    const data = await fetchCredentials();
    sshKey.value = data.ssh_key;
    accounts.value = data.accounts;
    platforms.value = data.platforms;
  } catch (e) {
    loadError.value = e.message;
  } finally {
    loading.value = false;
  }
}

async function act(action) {
  busy.value = true;
  actionError.value = '';
  try {
    await action();
  } catch (e) {
    actionError.value = e.message;
  } finally {
    busy.value = false;
  }
}

const createSshKey = () => act(async () => { sshKey.value = await generateSshKey(); });
const importKey = () => act(async () => {
  sshKey.value = await importSshKey(privateKeyInput.value);
  importing.value = false;
  privateKeyInput.value = '';
});
const removeSshKey = () => act(async () => {
  await deleteSshKey();
  sshKey.value = null;
});
const removeAccount = (account) => act(async () => {
  await deleteAccount(account.host);
  accounts.value = accounts.value.filter(a => a.host !== account.host);
});

async function copyPublicKey() {
  try {
    await navigator.clipboard.writeText(sshKey.value.public_key);
  } catch {
    return;
  }
  copied.value = true;
  setTimeout(() => { copied.value = false; }, COPIED_MS);
}

function openAccount(account = null) {
  accountError.value = '';
  const platform = account?.platform ?? platforms.value[0]?.id ?? 'github';
  accountForm.value = {
    existing: Boolean(account),
    platform,
    host: account?.host ?? platforms.value.find(p => p.id === platform)?.default_host ?? '',
    username: account?.username ?? '',
    secret: '',
  };
}

function applyDefaultHost() {
  accountForm.value.host = selectedPlatform.value?.default_host ?? '';
}

async function submitAccount() {
  busy.value = true;
  accountError.value = '';
  try {
    const { platform, host, username, secret } = accountForm.value;
    const data = await saveAccount({ platform, host, username, secret });
    accounts.value = data.accounts;
    accountForm.value = null;
  } catch (e) {
    accountError.value = e.message;
  } finally {
    busy.value = false;
  }
}

onMounted(load);
</script>

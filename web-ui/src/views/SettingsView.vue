<template>
  <div class="settings-page">
    <div class="settings-header">
      <h2>Settings</h2>
    </div>

    <section class="settings-section">
      <div class="settings-section__heading">
        <h3>Account</h3>
      </div>
      <div class="settings-account-card">
        <div class="settings-account-card__avatar" :style="{ background: avatarColor }" aria-hidden="true">
          {{ initials }}
        </div>
        <div class="settings-account-card__info">
          <span class="settings-account-card__name">{{ auth.user?.name }}</span>
          <span class="settings-account-card__email">{{ auth.user?.email }}</span>
        </div>
        <span v-if="auth.user?.provider" class="settings-account-card__provider">{{ auth.user.provider }}</span>
      </div>
    </section>

    <section class="settings-section">
      <div class="settings-section__heading">
        <h3>LLM API keys</h3>
        <p class="settings-section__description">Configure API keys for LLM providers that require user-provided credentials.</p>
      </div>

      <div v-if="loading" class="settings-loading">
        <LoadingSpinner text="Loading providers…" />
      </div>

      <div v-else-if="!providers.length" class="settings-empty">
        <svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"/></svg>
        <p>No providers require a user-provided API key.</p>
        <span>LLM providers are configured server-side.</span>
      </div>

      <div v-else class="settings-providers">
        <div
          v-for="p in providers"
          :key="p.id"
          class="provider-card"
          :class="{ 'provider-card--active': p.key_set }"
        >
          <div class="provider-card__icon" :class="`provider-card__icon--${p.kind}`">
            <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2z"/><path d="M2 12h20"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></svg>
          </div>
          <div class="provider-card__body">
            <div class="provider-card__header">
              <span class="provider-card__name">{{ p.name || p.kind }}</span>
              <span v-if="p.key_set" class="provider-status provider-status--set">
                <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12"/></svg>
                Key set
              </span>
              <span v-else class="provider-status provider-status--unset">
                <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
                No key
              </span>
            </div>
            <div class="provider-card__meta">
              <span class="provider-card__kind">{{ p.kind }}</span>
              <span v-if="p.updated_at" class="provider-card__updated">Updated {{ formatDate(p.updated_at) }}</span>
            </div>
          </div>
          <div class="provider-card__actions">
            <button
              v-if="!p.key_set"
              class="p-button--positive is-dense"
              type="button"
              @click="openEdit(p)"
            >Set key</button>
            <template v-else>
              <button
                class="provider-icon-btn"
                type="button"
                aria-label="Update API key"
                title="Update API key"
                @click="openEdit(p)"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>
              </button>
              <button
                class="provider-icon-btn provider-icon-btn--danger"
                type="button"
                aria-label="Remove API key"
                title="Remove API key"
                :disabled="deleting === p.id"
                @click="removeKey(p)"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
              </button>
            </template>
          </div>
        </div>
      </div>
    </section>

    <div v-if="editing" class="modal" @click.self="closeEdit">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="closeEdit">✕</button>
        <h3>{{ editing.key_set ? 'Update' : 'Set' }} API key for {{ editing.name || editing.kind }}</h3>
        <p class="modal-lede">Your key is encrypted at rest and never shared with other users.</p>
        <div class="form-group">
          <label for="api-key-input">API Key</label>
          <input
            id="api-key-input"
            v-model="keyInput"
            type="password"
            placeholder="Paste your API key here"
            autocomplete="off"
          />
        </div>
        <p v-if="editError" class="auth-error">{{ editError }}</p>
        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" @click="closeEdit">Cancel</button>
          <button
            class="p-button--positive is-dense"
            type="button"
            :disabled="!keyInput || saving"
            @click="saveKey"
          >{{ saving ? 'Saving…' : 'Save' }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue';
import { useLlmStore } from '../stores/llm.js';
import { useAuthStore } from '../stores/auth.js';
import { setLlmUserKey, deleteLlmUserKey } from '../lib/api.js';
import LoadingSpinner from '../components/deployment/LoadingSpinner.vue';

const llm = useLlmStore();
const auth = useAuthStore();

const providers = ref([]);
const editing = ref(null);
const keyInput = ref('');
const editError = ref('');
const saving = ref(false);
const deleting = ref(null);
const loading = ref(true);

const initials = computed(() => {
  const name = auth.user?.name || '';
  const parts = name.split(' ').filter(Boolean);
  if (parts.length === 0) return '?';
  if (parts.length === 1) return parts[0][0].toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
});

const avatarColor = computed(() => {
  const name = auth.user?.name || auth.user?.email || '?';
  const colors = ['#e95420', '#772953', '#0066cc', '#2d8a3e', '#5b3eb0', '#c7162b', '#335c81'];
  const hash = [...name].reduce((a, c) => a + c.charCodeAt(0), 0);
  return colors[hash % colors.length];
});

function formatDate(dateStr) {
  try {
    return new Date(dateStr).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  } catch {
    return dateStr;
  }
}

function openEdit(provider) {
  editing.value = provider;
  keyInput.value = '';
  editError.value = '';
}

function closeEdit() {
  editing.value = null;
  keyInput.value = '';
  editError.value = '';
}

async function saveKey() {
  if (!editing.value || !keyInput.value) return;
  saving.value = true;
  editError.value = '';
  try {
    await setLlmUserKey(editing.value.id, keyInput.value);
    await llm.loadUserKeys();
    await llm.load();
    providers.value = llm.userKeyProviders;
    closeEdit();
  } catch (e) {
    editError.value = e.message;
  } finally {
    saving.value = false;
  }
}

async function removeKey(provider) {
  deleting.value = provider.id;
  try {
    await deleteLlmUserKey(provider.id);
    await llm.loadUserKeys();
    await llm.load();
    providers.value = llm.userKeyProviders;
  } catch (e) {
    editError.value = e.message;
  } finally {
    deleting.value = null;
  }
}

onMounted(async () => {
  await llm.loadUserKeys();
  providers.value = llm.userKeyProviders;
  loading.value = false;
});
</script>

<style scoped>
.settings-page {
  flex: 1;
  overflow-y: auto;
  padding: 2rem 3rem;
  max-width: 900px;
  margin: 0 auto;
}

.settings-header h2 {
  margin: 0 0 2rem;
  font-size: 1.5rem;
  font-weight: 600;
}

.settings-section {
  margin-bottom: 2.5rem;
}

.settings-section__heading h3 {
  margin: 0 0 0.25rem;
  font-size: 1.125rem;
  font-weight: 600;
}

.settings-section__description {
  margin: 0 0 1.25rem;
  font-size: 0.875rem;
  color: var(--text-muted);
}

.settings-account-card {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 1.25rem;
  background: var(--bg-surface);
  border: 1px solid var(--border-color);
  border-radius: 4px;
}

.settings-account-card__avatar {
  width: 2.5rem;
  height: 2.5rem;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.875rem;
  font-weight: 600;
  color: #ffffff;
  flex-shrink: 0;
  letter-spacing: 0.03em;
}

.settings-account-card__info {
  display: flex;
  flex-direction: column;
  gap: 0.125rem;
  flex: 1;
  min-width: 0;
}

.settings-account-card__name {
  font-size: 0.9375rem;
  font-weight: 600;
  color: var(--text-primary);
}

.settings-account-card__email {
  font-size: 0.8125rem;
  color: var(--text-muted);
}

.settings-account-card__provider {
  font-size: 0.6875rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  font-weight: 600;
  color: var(--text-secondary);
  background: var(--bg-surface-alt);
  padding: 0.2rem 0.625rem;
  border-radius: 2px;
}

.settings-loading {
  display: flex;
  justify-content: center;
  padding: 2rem;
}

.settings-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
  padding: 3rem 1rem;
  color: var(--text-muted);
  text-align: center;
}

.settings-empty svg {
  opacity: 0.3;
  margin-bottom: 0.5rem;
}

.settings-empty p {
  margin: 0;
  font-size: 0.9375rem;
  font-weight: 500;
  color: var(--text-secondary);
}

.settings-empty span {
  font-size: 0.8125rem;
}

.settings-providers {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.provider-card {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 1rem 1.25rem;
  background: var(--bg-surface);
  border: 1px solid var(--border-color);
  border-radius: 4px;
  border-left: 3px solid var(--border-subtle);
  transition: border-color 0.15s;
}

.provider-card--active {
  border-left-color: #2d8a3e;
}

.provider-card__icon {
  width: 2.5rem;
  height: 2.5rem;
  border-radius: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: #ffffff;
  background: var(--text-muted);
}

.provider-card__icon--anthropic {
  background: #d97757;
}

.provider-card__icon--gemini {
  background: #4285f4;
}

.provider-card__icon--openai-compatible {
  background: #1a7f37;
}

.provider-card__body {
  flex: 1;
  min-width: 0;
}

.provider-card__header {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  margin-bottom: 0.25rem;
}

.provider-card__name {
  font-size: 0.9375rem;
  font-weight: 600;
  color: var(--text-primary);
}

.provider-status {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  font-size: 0.6875rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.03em;
  padding: 0.15rem 0.5rem;
  border-radius: 2px;
}

.provider-status--set {
  color: #1a7f37;
  background: rgba(26, 127, 55, 0.12);
}

.provider-status--unset {
  color: #c7162b;
  background: rgba(199, 22, 43, 0.08);
}

.provider-card__meta {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  font-size: 0.75rem;
  color: var(--text-muted);
}

.provider-card__kind {
  text-transform: capitalize;
}

.provider-card__actions {
  display: flex;
  gap: 0.375rem;
  flex-shrink: 0;
}

.provider-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 2rem;
  height: 2rem;
  padding: 0;
  border: 1px solid var(--border-color);
  border-radius: 2px;
  background: var(--bg-surface-alt);
  color: var(--text-secondary);
  cursor: pointer;
  line-height: 1;
  box-sizing: border-box;
  transition: background 0.15s, color 0.15s, border-color 0.15s;
}

.provider-icon-btn:hover:not(:disabled) {
  background: var(--bg-surface-hover);
  color: var(--text-primary);
}

.provider-icon-btn:disabled {
  opacity: 0.4;
  cursor: default;
}

.provider-icon-btn--danger:hover:not(:disabled) {
  color: #c7162b;
  border-color: #c7162b;
  background: rgba(199, 22, 43, 0.08);
}

.modal-actions {
  display: flex;
  gap: 0.5rem;
  justify-content: flex-end;
  margin-top: 0.5rem;
}
</style>

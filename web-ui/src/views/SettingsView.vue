<template>
  <div class="settings-view">
    <div class="p-panel">
      <div class="p-panel__header">
        <h2 class="p-panel__title">Settings</h2>
      </div>
      <div class="p-panel__content">
        <div class="p-card">
          <h3>LLM API Keys</h3>
          <p v-if="!providers.length" class="p-text--muted">
            No providers require a user-provided API key. LLM providers are configured server-side.
          </p>
          <table v-else class="p-table--mobile-card">
            <thead>
              <tr>
                <th>Provider</th>
                <th>Status</th>
                <th>Last updated</th>
                <th class="u-align--right">Actions</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="p in providers" :key="p.id">
                <td>
                  <strong>{{ p.name || p.kind }}</strong>
                  <br />
                  <small class="p-text--muted">{{ p.kind }}</small>
                </td>
                <td>
                  <span v-if="p.key_set" class="p-chip--positive">Key set</span>
                  <span v-else class="p-chip">No key</span>
                </td>
                <td>
                  <span v-if="p.updated_at" class="p-text--small">{{ formatDate(p.updated_at) }}</span>
                  <span v-else class="p-text--muted">—</span>
                </td>
                <td class="u-align--right">
                  <button
                    class="p-button--base is-dense"
                    type="button"
                    @click="openEdit(p)"
                  >{{ p.key_set ? 'Update' : 'Set key' }}</button>
                  <button
                    v-if="p.key_set"
                    class="p-button--negative is-dense"
                    type="button"
                    :disabled="deleting === p.id"
                    @click="removeKey(p)"
                  >Remove</button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>

    <div v-if="editing" class="modal" @click.self="closeEdit">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="closeEdit">✕</button>
        <h3>{{ editing.key_set ? 'Update' : 'Set' }} API key for {{ editing.name || editing.kind }}</h3>
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
          <button
            class="p-button--positive is-dense"
            type="button"
            :disabled="!keyInput || saving"
            @click="saveKey"
          >{{ saving ? 'Saving…' : 'Save' }}</button>
          <button
            class="p-button--base is-dense"
            type="button"
            @click="closeEdit"
          >Cancel</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue';
import { useLlmStore } from '../stores/llm.js';
import { setLlmUserKey, deleteLlmUserKey } from '../lib/api.js';

const llm = useLlmStore();

const providers = ref([]);
const editing = ref(null);
const keyInput = ref('');
const editError = ref('');
const saving = ref(false);
const deleting = ref(null);

function formatDate(dateStr) {
  try {
    return new Date(dateStr).toLocaleString();
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
  } catch (e) {
    editError.value = e.message;
  } finally {
    deleting.value = null;
  }
}

onMounted(async () => {
  await llm.loadUserKeys();
  providers.value = llm.userKeyProviders;
});
</script>

<style scoped>
.settings-view {
  padding: 1rem;
  max-width: 800px;
  margin: 0 auto;
}

.modal-actions {
  display: flex;
  gap: 0.5rem;
  margin-top: 1rem;
}

.p-table--mobile-card th,
.p-table--mobile-card td {
  padding: 0.5rem;
}
</style>

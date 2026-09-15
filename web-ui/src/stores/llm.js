import { defineStore } from 'pinia';
import { ref } from 'vue';
import { fetchLlmProviders, fetchLlmUserKeys, updateMe } from '../lib/api.js';
import { useAuthStore } from './auth.js';

export const useLlmStore = defineStore('llm', () => {
  const providers = ref([]);
  const selection = ref(null);
  const loading = ref(false);
  const userKeyProviders = ref([]);

  async function load() {
    loading.value = true;
    const [data, keysData] = await Promise.all([fetchLlmProviders(), fetchLlmUserKeys()]);
    providers.value = data.providers ?? [];
    userKeyProviders.value = keysData.providers ?? [];
    loading.value = false;
    if (!selection.value && providers.value.length) {
      const top = providers.value[0];
      selection.value = { providerId: top.id, model: top.default_model };
    }
  }

  async function loadUserKeys() {
    const keysData = await fetchLlmUserKeys();
    userKeyProviders.value = keysData.providers ?? [];
  }

  function setSelection(providerId, model = null) {
    selection.value = { providerId, model };
    updateMe({ last_llm_provider_id: providerId, last_llm_model: model }).catch(() => {});
  }

  function loadFromProfile() {
    const auth = useAuthStore();
    const providerId = auth.user?.last_llm_provider_id;
    if (!providerId) return;
    const exists = providers.value.some(p => p.id === providerId);
    if (!exists) return;
    selection.value = { providerId, model: auth.user?.last_llm_model ?? null };
  }

  return { providers, selection, loading, userKeyProviders, load, loadUserKeys, setSelection, loadFromProfile };
});

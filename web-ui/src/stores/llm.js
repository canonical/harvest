import { defineStore } from 'pinia';
import { ref } from 'vue';
import { fetchLlmProviders, updateMe } from '../lib/api.js';
import { useAuthStore } from './auth.js';

export const useLlmStore = defineStore('llm', () => {
  const providers = ref([]);
  const selection = ref(null);
  const loading = ref(false);

  async function load() {
    loading.value = true;
    const data = await fetchLlmProviders();
    providers.value = data.providers ?? [];
    loading.value = false;
    if (!selection.value && providers.value.length) {
      const top = providers.value[0];
      selection.value = { providerId: top.id, model: top.default_model };
    }
  }

  function setSelection(providerId, model = null) {
    selection.value = { providerId, model };
    updateMe({ last_llm_provider_id: providerId, last_llm_model: model }).catch(() => {});
  }

  function loadFromProfile() {
    const auth = useAuthStore();
    const providerId = auth.user?.last_llm_provider_id;
    if (!providerId) return;
    selection.value = { providerId, model: auth.user?.last_llm_model ?? null };
  }

  return { providers, selection, loading, load, setSelection, loadFromProfile };
});

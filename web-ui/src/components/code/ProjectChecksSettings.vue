<template>
  <details class="settings-section container-setup">
    <summary>
      <span class="container-setup__title">Checks</span>
      <span class="container-setup__status">{{ saved ? 'Custom' : 'Detected from the repository' }}</span>
    </summary>
    <p class="settings-section__description">
      Commands Harvest runs after each change in a session, one per line, such as tests, linters and type checkers.
      A <code>.harvest/checks</code> file in the repository takes precedence. Leave it empty to use the commands Harvest detects.
    </p>
    <textarea v-model="draft" class="code-setup" rows="4" spellcheck="false" aria-label="Checks" placeholder="python3 -m pytest&#10;pre-commit run --all-files" />
    <button class="p-button--positive is-dense" type="button" :disabled="!changed || saving" @click="save">{{ saving ? 'Saving…' : 'Save' }}</button>
  </details>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { saveProjectChecks } from '../../lib/code-api.js';
import { showToast } from '../../composables/useToasts.js';

const props = defineProps({
  projectId: { type: String, required: true },
  checks: { type: String, default: '' },
});

const saved = ref(props.checks ?? '');
const draft = ref(saved.value);
const saving = ref(false);
const changed = computed(() => draft.value.trim() !== saved.value.trim());

watch(() => props.checks, (checks) => {
  saved.value = checks ?? '';
  draft.value = saved.value;
});

async function save() {
  saving.value = true;
  try {
    await saveProjectChecks(props.projectId, draft.value);
    saved.value = draft.value.trim();
    draft.value = saved.value;
    showToast({ message: saved.value ? 'Checks saved' : 'Harvest will use the checks it detects' });
  } catch (e) {
    showToast({ message: e.message, kind: 'negative' });
  } finally {
    saving.value = false;
  }
}
</script>

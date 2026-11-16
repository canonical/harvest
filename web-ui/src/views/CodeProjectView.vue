<template>
  <div class="settings-page code-page">
    <p v-if="error" class="code-error" role="alert">{{ error }}</p>
    <template v-if="project">
      <div class="settings-header">
        <router-link to="/code" class="code-back">Code</router-link>
        <h1>{{ project.name }}</h1>
        <p class="u-text--muted">{{ project.url }}</p>
        <span class="p-chip project-index" :title="`Code graph: ${indexText}`">
          <span class="p-chip__lead">Code graph</span>
          <span class="p-chip__value">{{ indexText }}</span>
        </span>
      </div>

      <section class="settings-section">
        <div class="settings-section__heading">
          <h2>Start a session</h2>
          <p class="settings-section__description">Each session gets its own container and branch. Opening a branch you already work on brings you back to that session.</p>
        </div>
        <form class="code-session-form" @submit.prevent="start">
          <label for="code-branch">Branch</label>
          <input id="code-branch" v-model="branch" type="text" list="code-branches" placeholder="harvest/my-change" />
          <datalist id="code-branches">
            <option v-for="name in branches" :key="name" :value="name" />
          </datalist>
          <label for="code-base">From</label>
          <select id="code-base" v-model="baseBranch">
            <option v-for="name in baseChoices" :key="name" :value="name">{{ name }}</option>
          </select>
          <button class="p-button--positive is-dense" type="submit" :disabled="!branch.trim() || starting">{{ starting ? 'Starting…' : 'Start' }}</button>
        </form>
      </section>

      <section class="settings-section">
        <div class="settings-section__heading">
          <h2>Your sessions</h2>
        </div>
        <p v-if="!sessions.length" class="u-text--muted">No sessions yet.</p>
        <ul class="code-projects">
          <li v-for="session in sessions" :key="session.id">
            <router-link :to="`/code/sessions/${session.id}`" class="code-projects__item">
              <span class="code-projects__name">{{ session.branch }}</span>
              <span class="code-projects__meta">
                <span class="p-label" :class="`code-status--${session.status}`">{{ session.status }}</span>
                <span v-if="session.close_reason" class="u-text--muted">{{ session.close_reason }}</span>
              </span>
            </router-link>
          </li>
        </ul>
      </section>

      <details class="settings-section container-setup" :open="image?.status === 'failed'">
        <summary>
          <span class="container-setup__title">Container setup</span>
          <span class="container-setup__status" :class="`container-setup__status--${image?.status ?? 'none'}`">{{ imageStatus }}</span>
        </summary>
        <p class="settings-section__description">Runs from the repository root when Harvest prepares containers. A <code>.harvest/setup.sh</code> in the repository takes precedence. Leave it empty and the agent drafts one during your first session.</p>
        <div v-if="image?.status === 'failed' && image.log" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">The last setup failed.</p>
          </div>
          <div class="p-notification__actions">
            <button class="p-button--base is-dense u-no-margin--bottom" type="button" @click="showLog = !showLog">{{ showLog ? 'Hide log' : 'Show log' }}</button>
          </div>
          <pre v-if="showLog" class="code-log">{{ image.log }}</pre>
        </div>
        <textarea v-model="setupScript" class="code-setup" rows="8" spellcheck="false" aria-label="Setup script" placeholder="apt-get install -y …&#10;npm ci" />
        <button class="p-button--positive is-dense" type="button" :disabled="!setupChanged || saving" @click="saveSetup">{{ saving ? 'Saving…' : 'Save' }}</button>
      </details>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { useRouter } from 'vue-router';
import { getCodeProject, indexSummary, openCodeSession, saveSetupScript } from '../lib/code-api.js';
import { showToast } from '../composables/useToasts.js';

const props = defineProps({ id: { type: String, required: true } });
const router = useRouter();

const project = ref(null);
const branches = ref([]);
const sessions = ref([]);
const image = ref(null);
const index = ref(null);
const branch = ref('');
const baseBranch = ref('');
const setupScript = ref('');
const starting = ref(false);
const saving = ref(false);
const showLog = ref(false);
const error = ref('');

const baseChoices = computed(() => {
  const names = new Set(branches.value);
  if (project.value?.default_branch) names.add(project.value.default_branch);
  return [...names];
});
const indexText = computed(() => indexSummary(index.value));
const setupChanged = computed(() => setupScript.value !== (project.value?.setup_script ?? ''));
const imageStatus = computed(() => ({
  ready: 'Ready',
  building: 'Being prepared…',
  failed: 'Setup failed',
}[image.value?.status] ?? 'Not prepared yet'));

async function load() {
  error.value = '';
  try {
    const data = await getCodeProject(props.id);
    project.value = data.project;
    branches.value = data.branches ?? [];
    sessions.value = data.sessions ?? [];
    image.value = data.image;
    index.value = data.index;
    setupScript.value = data.project.setup_script ?? '';
    baseBranch.value = data.project.default_branch ?? branches.value[0] ?? '';
  } catch (e) {
    error.value = e.message;
  }
}

async function start() {
  starting.value = true;
  error.value = '';
  try {
    const session = await openCodeSession(props.id, branch.value.trim(), baseBranch.value);
    router.push(`/code/sessions/${session.id}`);
  } catch (e) {
    error.value = e.message;
  } finally {
    starting.value = false;
  }
}

async function saveSetup() {
  saving.value = true;
  try {
    await saveSetupScript(props.id, setupScript.value);
    project.value = { ...project.value, setup_script: setupScript.value.trim() || null };
    setupScript.value = project.value.setup_script ?? '';
    showToast({ message: 'Setup script saved; new containers will use it' });
  } catch (e) {
    error.value = e.message;
  } finally {
    saving.value = false;
  }
}

watch(() => props.id, load, { immediate: true });
</script>

<template>
  <div class="settings-page code-page">
    <p v-if="error" class="code-error" role="alert">{{ error }}</p>
    <template v-if="project">
      <div class="settings-header">
        <router-link :to="{ path: '/code', query: { project: id } }" class="code-back">Sessions</router-link>
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
        <SessionBranchForm :key="id" :project-id="id" :branches="branches" :default-branch="project.default_branch ?? ''" />
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
        <p class="settings-section__description">Runs from the repository root when Harvest prepares containers. A <code>.harvest/setup.sh</code> in the repository takes precedence. Leave it empty and Harvest installs what the repository's files call for, such as Python packages, npm dependencies or a Rust or Go toolchain.</p>
        <div v-if="image?.status === 'failed' && image.log" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">The last setup failed.</p>
          </div>
          <div class="p-notification__actions">
            <button class="p-button--base is-dense u-no-margin--bottom" type="button" @click="showLog = !showLog">{{ showLog ? 'Hide log' : 'Show log' }}</button>
          </div>
          <pre v-if="showLog" class="code-log">{{ image.log }}</pre>
        </div>
        <div v-if="learnedSetup.length" class="learned-setup">
          <p class="learned-setup__title">Commands that installed packages in your sessions</p>
          <pre class="code-log">{{ learnedSetup.join('\n') }}</pre>
          <button class="p-button--base is-dense" type="button" @click="adoptLearned">Add them to the setup script</button>
        </div>
        <textarea v-model="setupScript" class="code-setup" rows="8" spellcheck="false" aria-label="Setup script" placeholder="apt-get install -y …&#10;npm ci" />
        <button class="p-button--positive is-dense" type="button" :disabled="!setupChanged || saving" @click="saveSetup">{{ saving ? 'Saving…' : 'Save' }}</button>
      </details>
      <ProjectChecksSettings :project-id="id" :checks="project.checks ?? ''" />
    </template>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { getCodeProject, indexSummary, saveSetupScript } from '../lib/code-api.js';
import SessionBranchForm from '../components/code/SessionBranchForm.vue';
import ProjectChecksSettings from '../components/code/ProjectChecksSettings.vue';
import { showToast } from '../composables/useToasts.js';

const props = defineProps({ id: { type: String, required: true } });

const project = ref(null);
const branches = ref([]);
const sessions = ref([]);
const image = ref(null);
const index = ref(null);
const setupScript = ref('');
const learnedSetup = ref([]);
const saving = ref(false);
const showLog = ref(false);
const error = ref('');

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
    learnedSetup.value = data.learned_setup ?? [];
  } catch (e) {
    error.value = e.message;
  }
}

function adoptLearned() {
  const current = setupScript.value.trimEnd();
  setupScript.value = [current, ...learnedSetup.value].filter(Boolean).join('\n');
}

async function saveSetup() {
  saving.value = true;
  try {
    await saveSetupScript(props.id, setupScript.value);
    project.value = { ...project.value, setup_script: setupScript.value.trim() || null };
    setupScript.value = project.value.setup_script ?? '';
    learnedSetup.value = [];
    showToast({ message: 'Setup script saved; new containers will use it' });
  } catch (e) {
    error.value = e.message;
  } finally {
    saving.value = false;
  }
}

watch(() => props.id, load, { immediate: true });
</script>

<template>
  <AppModal :title="project ? `New session in ${project.name}` : 'New session'" size="wide" @close="emit('close')">
    <template v-if="project">
      <p class="modal-lede">{{ project.url }}</p>
      <SessionBranchForm :project-id="project.id" :branches="project.branches" :default-branch="project.defaultBranch" autofocus />
    </template>

    <p v-else-if="preparing" class="u-text--muted" role="status">Checking access to {{ preparing }}…</p>

    <template v-else>
      <input v-model="search" type="search" class="code-choices__search" placeholder="Search projects" aria-label="Search projects" />
      <p v-if="loading" class="u-text--muted">Loading projects…</p>
      <p v-else-if="!visibleChoices.length" class="u-text--muted">{{ search.trim() ? 'No project matches.' : 'No ingested projects yet.' }}</p>
      <ul class="code-choices">
        <li v-for="choice in visibleChoices" :key="choice.name">
          <button type="button" class="code-choices__item" @click="prepare(choice.name, () => openCodeProject(choice.url))">
            <span class="code-choices__name">{{ choice.name }}</span>
            <span class="code-choices__url">{{ choice.url }}</span>
            <span v-if="choice.visibility === 'private'" class="p-label">private</span>
          </button>
        </li>
      </ul>
      <form class="code-choices__new" @submit.prevent="prepare(url.trim(), () => openCodeProject(url.trim()))">
        <label for="new-session-url">Not listed? Paste the URL of a git repository. Private repositories use the SSH key or token from your Git access settings.</label>
        <div class="code-open">
          <input id="new-session-url" v-model="url" type="text" placeholder="https://github.com/owner/repository" />
          <button class="p-button is-dense" type="submit" :disabled="!url.trim()">Open</button>
        </div>
      </form>
    </template>

    <p v-if="error" class="code-error" role="alert">{{ error }}</p>

    <template v-if="project" #actions>
      <button class="p-button--base is-dense" type="button" @click="project = null">Choose another project</button>
    </template>
  </AppModal>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import AppModal from '../ui/AppModal.vue';
import SessionBranchForm from './SessionBranchForm.vue';
import { fetchRepositories } from '../../lib/api.js';
import { getCodeProject, openCodeProject } from '../../lib/code-api.js';
import { repositoryChoices, searchChoices } from '../../lib/code-sessions.js';

const props = defineProps({
  projectId: { type: String, default: null },
  projects: { type: Array, default: () => [] },
});
const emit = defineEmits(['close']);

const choices = ref([]);
const loading = ref(true);
const search = ref('');
const url = ref('');
const preparing = ref('');
const project = ref(null);
const error = ref('');

const visibleChoices = computed(() => searchChoices(choices.value, search.value));

async function prepare(label, openProject) {
  preparing.value = label;
  error.value = '';
  try {
    const { id } = await openProject();
    const data = await getCodeProject(id);
    project.value = {
      id,
      name: data.project.name,
      url: data.project.url,
      branches: data.branches ?? [],
      defaultBranch: data.project.default_branch ?? '',
    };
  } catch (e) {
    error.value = e.message;
  } finally {
    preparing.value = '';
  }
}

async function loadChoices() {
  choices.value = repositoryChoices(await fetchRepositories(), props.projects);
  loading.value = false;
}

onMounted(() => {
  loadChoices();
  if (props.projectId) {
    const name = props.projects.find((candidate) => candidate.id === props.projectId)?.name ?? 'the project';
    prepare(name, () => ({ id: props.projectId }));
  }
});
</script>

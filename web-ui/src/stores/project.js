import { defineStore } from 'pinia';
import { ref, computed } from 'vue';
import { fetchProjects as apiFetchProjects, createProject as apiCreateProject, fetchMyGroups, updateMe } from '../lib/api.js';

const LS_KEY = 'harvest_selected_project_id';

export const useProjectStore = defineStore('project', () => {
  const projects        = ref([]);
  const selectedProject = ref(null);
  let   _pendingId      = null;

  const selectedProjectId = computed(() => selectedProject.value?.id ?? null);

  async function fetchProjects() {
    try {
      projects.value = await apiFetchProjects();
      _flushPending();
    } catch {}
  }

  function selectProject(project) {
    selectedProject.value = project ?? null;
    if (project?.id) {
      localStorage.setItem(LS_KEY, project.id);
      updateMe({ last_project_id: project.id }).catch(() => {});
    } else {
      localStorage.removeItem(LS_KEY);
    }
  }

  function clearProject() {
    selectedProject.value = null;
    localStorage.removeItem(LS_KEY);
  }

  function selectProjectById(id) {
    if (!id) return;
    const found = projects.value.find(p => p.id === id);
    if (found) {
      selectedProject.value = found;
    } else {
      _pendingId = id;
    }
  }

  function _flushPending() {
    const id = _pendingId ?? localStorage.getItem(LS_KEY);
    if (!id) return;
    const found = projects.value.find(p => p.id === id);
    if (found) {
      selectedProject.value = found;
      _pendingId = null;
    }
  }

  async function createProject(body) {
    const project = await apiCreateProject(body);
    await fetchProjects();
    return project;
  }

  async function fetchGroups() {
    try { return await fetchMyGroups(); } catch { return []; }
  }

  return {
    projects, selectedProject, selectedProjectId,
    fetchProjects, selectProject, clearProject, selectProjectById, _flushPending,
    createProject, fetchGroups,
  };
});

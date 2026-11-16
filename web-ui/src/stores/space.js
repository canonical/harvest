import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import { useProjectStore } from './project.js';
import { MODE, usesProjects } from '../lib/modes.js';

export const useSpaceStore = defineStore('space', () => {
  const project = useProjectStore();
  const mode = ref(MODE.CHAT);

  const projectId = computed(() => (usesProjects(mode.value) ? project.selectedProjectId : null));
  const isPersonal = computed(() => mode.value === MODE.CHAT);

  function enter(next) {
    if (next && next !== mode.value) mode.value = next;
  }

  return { mode, projectId, isPersonal, enter };
});

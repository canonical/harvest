<template>
  <div class="deployments-page">
    <div v-if="!projectId" class="no-project-state">
      <p>Select a project to view its deployments.</p>
    </div>

    <template v-else>
      <div class="deployments-header">
        <h2>Deployments</h2>
        <span v-if="deployments.length" class="deployments-header__count">{{ deployments.length }}</span>
        <button class="p-button--positive new-deployment-btn is-dense" type="button" @click="openNewModal">
          + New deployment
        </button>
      </div>

      <div v-if="loading" class="deployments-list-loading">
        <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
      </div>
      <p v-else-if="!deployments.length" class="deployments-list-empty">
        No deployments yet. Start one to design, prepare, and run infrastructure for a customer.
      </p>
      <table v-else class="deployments-table">
        <thead>
          <tr>
            <th>Name</th>
            <th>Product</th>
            <th>Status</th>
            <th>Updated</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="d in deployments"
            :key="d.id"
            class="deployments-table__row"
            :data-testid="`deployment-row-${d.id}`"
            @click="openDeployment(d.id)"
          >
            <td>{{ d.name }}</td>
            <td>{{ d.template ? d.template.name : 'From scratch' }}</td>
            <td>
              <span class="infra-state-badge" :class="infraStateClass(d.infra_state)">
                {{ infraStateLabel(d.infra_state) }}
              </span>
            </td>
            <td>{{ formatDate(d.updated_at) }}</td>
          </tr>
        </tbody>
      </table>
    </template>

    <div v-if="newModalOpen" class="modal" @click.self="closeNewModal">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="closeNewModal">✕</button>
        <h3>New deployment</h3>

        <div class="form-group">
          <label for="deployment-name">Name</label>
          <input id="deployment-name" v-model="newName" type="text" placeholder="e.g. Acme Corp rollout" />
        </div>

        <div class="form-group">
          <label for="deployment-template">Product template</label>
          <select id="deployment-template" v-model="newTemplateId">
            <option value="">Start from scratch</option>
            <option v-for="t in templates" :key="t.id" :value="t.id">{{ t.name }}</option>
          </select>
        </div>

        <div class="form-group">
          <label for="deployment-env">Customer environment</label>
          <textarea
            id="deployment-env"
            v-model="newEnvDescription"
            rows="4"
            placeholder="Describe the customer's network, constraints, and anything relevant to the deployment"
          ></textarea>
        </div>

        <div v-if="createError" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ createError }}</p>
          </div>
        </div>

        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" @click="closeNewModal">Cancel</button>
          <button class="p-button--positive is-dense" type="button" :disabled="!canCreate || creating" @click="submitNew">
            {{ creating ? 'Creating…' : 'Create' }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useProjectStore } from '../stores/project.js';
import { listProjectDeployments, createProjectDeployment, listGroupTemplates } from '../lib/api.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const router  = useRouter();
const project = useProjectStore();

const deployments = ref([]);
const loading      = ref(false);
const templates    = ref([]);

const newModalOpen     = ref(false);
const newName           = ref('');
const newTemplateId     = ref('');
const newEnvDescription = ref('');
const creating          = ref(false);
const createError       = ref(null);

const canCreate = computed(() => newName.value.trim().length > 0);

const INFRA_STATE_LABELS = {
  none: 'Not deployed', up: 'Up', broken: 'Broken', destroyed: 'Destroyed', destroy_failed: 'Destroy failed',
};

function infraStateLabel(state) {
  return INFRA_STATE_LABELS[state] ?? state;
}

function infraStateClass(state) {
  if (state === 'up') return 'infra-state-badge--up';
  if (state === 'broken' || state === 'destroy_failed') return 'infra-state-badge--broken';
  if (state === 'destroyed') return 'infra-state-badge--destroyed';
  return 'infra-state-badge--none';
}

function formatDate(iso) {
  return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

async function loadList() {
  if (!props.projectId) return;
  loading.value = true;
  try {
    deployments.value = await listProjectDeployments(props.projectId);
  } catch {
    deployments.value = [];
  }
  loading.value = false;
}

async function loadTemplates() {
  const groupId = project.selectedProject?.group_id;
  if (!groupId) {
    templates.value = [];
    return;
  }
  try {
    templates.value = await listGroupTemplates(groupId);
  } catch {
    templates.value = [];
  }
}

function openNewModal() {
  newModalOpen.value     = true;
  newName.value           = '';
  newTemplateId.value     = '';
  newEnvDescription.value = '';
  createError.value       = null;
  loadTemplates();
}

function closeNewModal() {
  newModalOpen.value = false;
}

async function submitNew() {
  if (!canCreate.value || !props.projectId) return;
  creating.value    = true;
  createError.value = null;
  try {
    const created = await createProjectDeployment(props.projectId, {
      name: newName.value.trim(),
      environment_description: newEnvDescription.value,
      product_template_id: newTemplateId.value || null,
    });
    newModalOpen.value = false;
    router.push(`/deployments/${created.id}`);
  } catch (e) {
    createError.value = e.message || 'Failed to create deployment';
  } finally {
    creating.value = false;
  }
}

function openDeployment(id) {
  router.push(`/deployments/${id}`);
}

watch(() => props.projectId, () => {
  deployments.value = [];
  if (props.projectId) loadList();
}, { immediate: true });
</script>

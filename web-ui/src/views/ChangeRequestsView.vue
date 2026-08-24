<template>
  <div class="change-requests-page">
    <div v-if="!projectId" class="no-project-state">
      <p>Select a project to view its change requests.</p>
    </div>

    <template v-else>
      <div class="change-requests-header">
        <h2>Change Requests</h2>
      </div>

      <div v-if="error" class="p-notification--negative">
        <div class="p-notification__content"><p class="p-notification__message">{{ error }}</p></div>
      </div>

      <div v-if="loading" class="change-requests-list-loading">
        <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
      </div>

      <div v-else class="change-requests-board" data-testid="change-requests-board">
        <div
          v-for="col in columns"
          :key="col.id"
          class="issues-column"
          :class="{ 'issues-column--drop-disabled': dragging && !isValidTransition(dragging.status, col.id) }"
          :data-testid="`issues-column-${col.id}`"
          @dragover.prevent="onDragOver(col.id)"
          @drop.prevent="onDrop(col.id)"
        >
          <div class="issues-column__header">
            <h3>{{ col.label }}</h3>
            <span class="issues-column__count">{{ crsByStatus[col.id].length }}</span>
          </div>

          <div class="issues-column__list">
            <div
              v-for="cr in crsByStatus[col.id]"
              :key="cr.id"
              class="issue-card"
              data-testid="issue-card"
              draggable="true"
              @dragstart="onDragStart(cr)"
              @dragend="onDragEnd"
              @click="openChangeRequest(cr.id)"
            >
              <div class="issue-card__title">{{ cr.title }}</div>
              <div class="issue-card__deployment">{{ cr.deployment?.name }}</div>
              <span class="p-chip issue-card__kind" :data-testid="`cr-kind-${cr.id}`">{{ cr.kind }}</span>

              <div class="issue-card__actions" @click.stop>
                <button
                  v-for="target in nextStatusOptions(cr.status)"
                  :key="target"
                  class="p-button--base is-dense"
                  type="button"
                  :data-testid="`move-issue-${cr.id}-${target}`"
                  @click="moveCR(cr, target)"
                >{{ actionLabel(target) }}</button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { listChangeRequests, discardChangeRequest } from '../lib/api.js';
import { CHANGE_REQUEST_STATUSES, isValidTransition, nextStatusOptions } from '../lib/change-request-transitions.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const route  = useRoute();
const router = useRouter();

const crs      = ref([]);
const loading  = ref(false);
const error    = ref(null);
const dragging = ref(null);

const COLUMN_LABELS = { open: 'Open', in_review: 'In Review', applied: 'Applied', discarded: 'Discarded' };
const ACTION_LABELS = { in_review: 'Review', applied: 'Apply', discarded: 'Discard' };

const columns = CHANGE_REQUEST_STATUSES.map(id => ({ id, label: COLUMN_LABELS[id] }));

function actionLabel(target) {
  return ACTION_LABELS[target] ?? target;
}

const crsByStatus = computed(() => {
  const grouped = Object.fromEntries(CHANGE_REQUEST_STATUSES.map(s => [s, []]));
  for (const cr of crs.value) {
    (grouped[cr.status] ?? (grouped[cr.status] = [])).push(cr);
  }
  return grouped;
});

async function load() {
  if (!props.projectId) return;
  loading.value = true;
  error.value = null;
  try {
    crs.value = await listChangeRequests(props.projectId, { deploymentId: route.query.deployment });
  } catch (e) {
    crs.value = [];
    error.value = e.message || 'Failed to load change requests';
  }
  loading.value = false;
}

function onDragStart(cr) { dragging.value = cr; }
function onDragEnd()      { dragging.value = null; }
function onDragOver()     {}

function onDrop(columnId) {
  const cr = dragging.value;
  dragging.value = null;
  if (!cr || !isValidTransition(cr.status, columnId)) return;
  moveCR(cr, columnId);
}

async function moveCR(cr, newStatus) {
  const previousStatus = cr.status;
  if (newStatus === 'discarded') {
    cr.status = newStatus;
    try {
      await discardChangeRequest(props.projectId, cr.id);
    } catch (e) {
      cr.status = previousStatus;
      error.value = e.message || 'Failed to discard change request';
    }
  }
}

function openChangeRequest(id) {
  router.push(`/change-requests/${id}`);
}

watch(() => [props.projectId, route.query.deployment], () => {
  crs.value = [];
  load();
}, { immediate: true });
</script>

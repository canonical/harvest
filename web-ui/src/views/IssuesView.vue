<template>
  <div class="issues-page">
    <div v-if="!projectId" class="no-project-state">
      <p>Select a project to view its issues.</p>
    </div>

    <template v-else>
      <div class="issues-header">
        <h2>Issues</h2>
      </div>

      <div v-if="error" class="p-notification--negative">
        <div class="p-notification__content"><p class="p-notification__message">{{ error }}</p></div>
      </div>

      <div v-if="loading" class="issues-list-loading">
        <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
      </div>

      <div v-else class="issues-board" data-testid="issues-board">
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
            <span class="issues-column__count">{{ issuesByStatus[col.id].length }}</span>
          </div>

          <div class="issues-column__list">
            <div
              v-for="issue in issuesByStatus[col.id]"
              :key="issue.id"
              class="issue-card"
              data-testid="issue-card"
              draggable="true"
              @dragstart="onDragStart(issue)"
              @dragend="onDragEnd"
              @click="openIssue(issue.id)"
            >
              <div class="issue-card__title">{{ issue.title }}</div>
              <div class="issue-card__deployment">{{ issue.deployment?.name }}</div>
              <div v-if="issue.has_proposed_solution" class="p-chip issue-card__has-fix">Fix proposed</div>

              <div class="issue-card__actions" @click.stop>
                <button
                  v-for="target in nextStatusOptions(issue.status)"
                  :key="target"
                  class="p-button--base is-dense"
                  type="button"
                  :data-testid="`move-issue-${issue.id}-${target}`"
                  @click="moveIssue(issue, target)"
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
import { listProjectIssues, updateIssueStatus } from '../lib/api.js';
import { ISSUE_STATUSES, isValidTransition, nextStatusOptions } from '../lib/issue-transitions.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const route  = useRoute();
const router = useRouter();

const issues  = ref([]);
const loading = ref(false);
const error   = ref(null);
const dragging = ref(null);

const COLUMN_LABELS = { untriaged: 'Untriaged', in_progress: 'In Progress', fixed: 'Fixed', rejected: 'Rejected' };
const ACTION_LABELS = { in_progress: 'Start work', fixed: 'Mark fixed', rejected: 'Reject' };

const columns = ISSUE_STATUSES.map(id => ({ id, label: COLUMN_LABELS[id] }));

function actionLabel(target) {
  return ACTION_LABELS[target] ?? target;
}

const issuesByStatus = computed(() => {
  const grouped = Object.fromEntries(ISSUE_STATUSES.map(s => [s, []]));
  for (const issue of issues.value) {
    (grouped[issue.status] ?? (grouped[issue.status] = [])).push(issue);
  }
  return grouped;
});

async function load() {
  if (!props.projectId) return;
  loading.value = true;
  error.value = null;
  try {
    issues.value = await listProjectIssues(props.projectId, { deploymentId: route.query.deployment });
  } catch (e) {
    issues.value = [];
    error.value = e.message || 'Failed to load issues';
  }
  loading.value = false;
}

function onDragStart(issue) {
  dragging.value = issue;
}

function onDragEnd() {
  dragging.value = null;
}

function onDragOver(columnId) {
  if (dragging.value && !isValidTransition(dragging.value.status, columnId)) return;
}

function onDrop(columnId) {
  const issue = dragging.value;
  dragging.value = null;
  if (!issue || !isValidTransition(issue.status, columnId)) return;
  moveIssue(issue, columnId);
}

async function moveIssue(issue, newStatus) {
  const previousStatus = issue.status;
  issue.status = newStatus;
  try {
    await updateIssueStatus(props.projectId, issue.id, newStatus);
  } catch (e) {
    issue.status = previousStatus;
    error.value = e.message || 'Failed to move issue';
  }
}

function openIssue(id) {
  router.push(`/issues/${id}`);
}

watch(() => [props.projectId, route.query.deployment], () => {
  issues.value = [];
  load();
}, { immediate: true });
</script>

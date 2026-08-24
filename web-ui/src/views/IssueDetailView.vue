<template>
  <div class="issue-detail-page">
    <div v-if="loading" class="issue-detail-loading">
      <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
    </div>

    <template v-else-if="issue">
      <div class="issue-detail-header">
        <h2>{{ issue.title }}</h2>
        <span class="issue-status-badge" :class="`issue-status-badge--${issue.status}`" data-testid="issue-status-badge">
          {{ statusLabel(issue.status) }}
        </span>
        <router-link to="/deploy" class="issue-detail-header__deployment-link" data-testid="view-deployment-link">
          View deployment: {{ issue.deployment.name }}
        </router-link>

        <div class="issue-detail-header__actions">
          <button
            v-for="target in nextStatusOptions(issue.status)"
            :key="target"
            class="p-button--base is-dense"
            type="button"
            :data-testid="`move-issue-${target}`"
            @click="moveStatus(target)"
          >{{ actionLabel(target) }}</button>
        </div>
      </div>

      <div v-if="actionError" class="p-notification--negative">
        <div class="p-notification__content"><p class="p-notification__message">{{ actionError }}</p></div>
      </div>

      <div class="issue-detail-body">
        <div class="issue-detail-main">
          <div class="issue-detail-description" data-testid="issue-description" v-html="renderMarkdown(issue.description || '', {}, {})"></div>

          <section class="issue-detail-section">
            <h3>Related runs</h3>
            <RunHistory :runs="issue.runs" />
          </section>

          <section v-if="issue.proposed_files" class="issue-detail-section" data-testid="proposed-solution-panel">
            <h3>Proposed solution</h3>
            <p class="issue-detail-proposed-summary">{{ issue.proposed_solution_summary }}</p>
            <DiffView :before="bundleFiles" :after="issue.proposed_files" />
            <button
              class="p-button--positive is-dense"
              data-testid="apply-solution-btn"
              type="button"
              :disabled="!selectedAgentId || applying"
              @click="applySolution"
            >{{ applying ? 'Applying…' : 'Apply and redeploy' }}</button>
          </section>

          <section class="issue-detail-section">
            <h3>Activity</h3>
            <IssueCommentList :comments="issue.comments" @post-comment="postComment" />
          </section>

          <section class="issue-detail-section">
            <h3>Investigate</h3>
            <IssueChat
              :project-id="projectId"
              :issue-id="issue.id"
              :history="issue.chat_messages"
              :proposed-files="issue.proposed_files"
              :proposed-summary="issue.proposed_solution_summary"
              :before-files="bundleFiles"
              @refresh="loadIssue"
              @approve="applySolution"
            />
          </section>
        </div>

        <aside class="issue-detail-sidebar">
          <div v-if="agents.length > 1" class="form-group">
            <label for="issue-agent-select">Agent</label>
            <select id="issue-agent-select" v-model="selectedAgentId" data-testid="issue-agent-select">
              <option value="" disabled>Select an agent</option>
              <option v-for="a in agents" :key="a.id" :value="a.id">{{ a.hostname }}</option>
            </select>
          </div>
          <button
            class="p-button--positive is-dense"
            data-testid="issue-redeploy-btn"
            type="button"
            :disabled="!selectedAgentId || redeploying"
            @click="redeploy"
          >{{ redeploying ? 'Redeploying…' : 'Redeploy' }}</button>
        </aside>
      </div>
    </template>

    <div v-else class="issue-detail-error">Failed to load issue.</div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { useRoute } from 'vue-router';
import RunHistory from '../components/deployment/RunHistory.vue';
import DiffView from '../components/deployment/DiffView.vue';
import IssueCommentList from '../components/deployment/IssueCommentList.vue';
import IssueChat from '../components/deployment/IssueChat.vue';
import { renderMarkdown } from '../lib/markdown.js';
import { nextStatusOptions } from '../lib/change-request-transitions.js';
import {
  getChangeRequest, discardChangeRequest, createChangeRequestComment, applyChangeRequest, redeployFromIssue,
  getProjectDeploymentSingle, getArtifact, listProjectAgents,
} from '../lib/api.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const route = useRoute();
const issueId = computed(() => route.params.id);

const issue       = ref(null);
const bundleFiles = ref({});
const agents      = ref([]);
const loading     = ref(false);
const actionError = ref(null);
const applying     = ref(false);
const redeploying  = ref(false);
const selectedAgentId = ref('');

const STATUS_LABELS = { open: 'Open', in_review: 'In Review', applied: 'Applied', discarded: 'Discarded' };
const ACTION_LABELS = { in_review: 'Review', applied: 'Apply', discarded: 'Discard' };

function statusLabel(status) {
  return STATUS_LABELS[status] ?? status;
}
function actionLabel(target) {
  return ACTION_LABELS[target] ?? target;
}

async function loadIssue() {
  if (!props.projectId || !issueId.value) return;
  issue.value = await getChangeRequest(props.projectId, issueId.value);
}

async function loadBundleFiles() {
  if (!issue.value?.deployment?.id) {
    bundleFiles.value = {};
    return;
  }
  try {
    const deployment = await getProjectDeploymentSingle(props.projectId);
    if (!deployment.terraform_bundle) {
      bundleFiles.value = {};
      return;
    }
    const artifact = await getArtifact(deployment.terraform_bundle.id);
    bundleFiles.value = JSON.parse(artifact.content || '{}');
  } catch {
    bundleFiles.value = {};
  }
}

async function load() {
  loading.value = true;
  try {
    await loadIssue();
    await Promise.all([
      loadBundleFiles(),
      listProjectAgents(props.projectId).then(a => { agents.value = a; }).catch(() => { agents.value = []; }),
    ]);
  } catch {
    issue.value = null;
  }
  loading.value = false;
}

watch(() => agents.value, (list) => {
  if (!selectedAgentId.value && list.length === 1) selectedAgentId.value = list[0].id;
});

async function moveStatus(target) {
  actionError.value = null;
  const previous = issue.value.status;
  issue.value.status = target;
  try {
    if (target === 'discarded') {
      await discardChangeRequest(props.projectId, issue.value.id);
    } else if (target === 'applied') {
      await applyChangeRequest(props.projectId, issue.value.id, { agent_id: selectedAgentId.value });
    }
  } catch (e) {
    issue.value.status = previous;
    actionError.value = e.message || 'Failed to move change request';
  }
}

async function postComment(text) {
  try {
    issue.value = await createChangeRequestComment(props.projectId, issue.value.id, text);
  } catch (e) {
    actionError.value = e.message || 'Failed to post comment';
  }
}

async function applySolution() {
  if (!selectedAgentId.value) return;
  applying.value = true;
  actionError.value = null;
  try {
    await applyChangeRequest(props.projectId, issue.value.id, { agent_id: selectedAgentId.value });
    await load();
  } catch (e) {
    actionError.value = e.message || 'Failed to apply the proposed solution';
  } finally {
    applying.value = false;
  }
}

async function redeploy() {
  if (!selectedAgentId.value) return;
  redeploying.value = true;
  actionError.value = null;
  try {
    await redeployFromIssue(props.projectId, issue.value.id, { agent_id: selectedAgentId.value });
    await load();
  } catch (e) {
    actionError.value = e.message || 'Failed to redeploy';
  } finally {
    redeploying.value = false;
  }
}

watch(() => [props.projectId, issueId.value], () => {
  issue.value = null;
  load();
}, { immediate: true });
</script>

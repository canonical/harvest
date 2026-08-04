<template>
  <div class="artifacts-page">
    <div v-if="!projectId && !routeArtifactId" class="no-project-state">
      <p>Select a project to view its artifacts.</p>
    </div>

    <template v-else>
      <div class="artifacts-header">
        <h2>Artifacts</h2>
        <span v-if="artifacts.length" class="artifacts-header__count">{{ artifacts.length }}</span>
      </div>

      <div class="artifacts-layout">
        <div class="artifacts-list">
          <div v-if="loading" class="artifacts-list-loading">
            <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
          </div>
          <p v-else-if="!artifacts.length" class="artifacts-list-empty">
            No artifacts yet. Ask the assistant to generate a document and it will show up here.
          </p>
          <button
            v-for="a in artifacts"
            :key="a.id"
            class="artifacts-list-item"
            :class="{ 'artifacts-list-item--active': a.id === selectedId }"
            @click="selectArtifact(a.id)"
          >
            <span class="artifacts-list-item__title">{{ a.title }}</span>
            <span class="artifacts-list-item__meta">
              <span class="artifact-kind-badge" :class="kindBadgeClass(a.kind)">{{ kindLabel(a.kind) }}</span>
              <span class="artifacts-list-item__date">{{ formatDate(a.created_at) }}</span>
            </span>
          </button>
        </div>

        <div class="artifacts-detail">
          <div v-if="!selectedId" class="artifacts-detail-empty">
            <p v-if="artifacts.length">Select an artifact on the left to view it.</p>
            <p v-else>Generated documents will appear here once the assistant creates one.</p>
          </div>
          <div v-else-if="contentLoading" class="artifacts-detail-loading">
            <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
          </div>
          <div v-else-if="selectedArtifact" class="artifacts-article">
            <div class="artifacts-article__header">
              <div class="artifacts-article__meta">
                <h2 class="artifacts-article__title">{{ selectedArtifact.title }}</h2>
                <div class="artifacts-article__subline">
                  <span class="artifact-kind-badge" :class="kindBadgeClass(selectedArtifact.kind)">{{ kindLabel(selectedArtifact.kind) }}</span>
                  <span class="artifacts-article__date">{{ createdByLabel(selectedArtifact.created_by) }} · {{ formatDate(selectedArtifact.created_at) }}</span>
                </div>
              </div>
              <div class="artifacts-article__actions">
                <button
                  v-if="isTerraformKind(selectedArtifact.kind)"
                  class="p-button--base run-on-agent-btn"
                  type="button"
                  @click="openRunModal"
                >
                  Run on agent
                </button>
                <a
                  class="p-button--positive artifact-download-btn"
                  :href="artifactDownloadUrl(selectedArtifact.id)"
                  download
                >
                  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
                  Download
                </a>
                <button
                  class="console-icon-btn console-icon-btn--danger delete-artifact-btn"
                  type="button"
                  title="Delete artifact"
                  aria-label="Delete artifact"
                  @click="confirmDeleteArtifact(selectedArtifact)"
                >
                  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/></svg>
                </button>
              </div>
            </div>
            <div class="artifacts-article__body" v-html="renderedContent" />
          </div>
          <div v-else class="artifacts-detail-error">Failed to load artifact.</div>
        </div>
      </div>
    </template>

    <div v-if="deletingArtifact" class="modal" @click.self="deletingArtifact = null">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="deletingArtifact = null">✕</button>
        <h3>Delete artifact</h3>
        <p>Delete <strong>{{ deletingArtifact.title }}</strong>? This cannot be undone.</p>
        <div class="modal-actions">
          <button class="p-button--base" type="button" @click="deletingArtifact = null">Cancel</button>
          <button class="p-button--negative" type="button" :disabled="submitting" @click="submitDeleteArtifact">Delete</button>
        </div>
      </div>
    </div>

    <div v-if="runModalOpen" class="modal" @click.self="closeRunModal">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="closeRunModal">✕</button>
        <h3>Run on agent</h3>

        <div class="form-group">
          <label for="run-agent-select">Agent</label>
          <select id="run-agent-select" v-model="selectedAgentId">
            <option value="" disabled>Select an agent</option>
            <option v-for="a in availableAgents" :key="a.id" :value="a.id">{{ a.hostname }}</option>
          </select>
        </div>

        <div class="form-group">
          <label for="run-action-select">Action</label>
          <select id="run-action-select" v-model="selectedAction">
            <option value="plan">Plan</option>
            <option value="apply">Apply</option>
            <option value="destroy">Destroy</option>
          </select>
        </div>

        <label v-if="selectedAction !== 'plan'" class="run-modal-confirm">
          <input v-model="confirmDangerous" type="checkbox" />
          This may create, change, or destroy real infrastructure.
        </label>

        <div v-if="runError" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ runError }}</p>
          </div>
        </div>

        <pre v-if="runResult" class="run-modal-result">{{ formattedRunResult }}</pre>

        <div class="modal-actions">
          <button class="p-button--base" type="button" @click="closeRunModal">Cancel</button>
          <button
            :class="selectedAction === 'plan' ? 'p-button--positive' : 'p-button--negative'"
            type="button"
            :disabled="!canSubmitRun || running"
            @click="submitRun"
          >
            {{ running ? 'Running…' : 'Run' }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { renderMarkdown } from '../lib/markdown.js';
import {
  listProjectArtifacts,
  getArtifact,
  deleteArtifact,
  artifactDownloadUrl,
  listProjectAgents,
  runTerraformArtifact,
} from '../lib/api.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const route  = useRoute();
const router = useRouter();

const artifacts        = ref([]);
const loading          = ref(false);
const selectedId       = ref(null);
const selectedArtifact = ref(null);
const contentLoading   = ref(false);
const deletingArtifact = ref(null);
const submitting       = ref(false);

const routeArtifactId = computed(() => route.params.id ?? null);

const runModalOpen     = ref(false);
const availableAgents  = ref([]);
const selectedAgentId  = ref('');
const selectedAction   = ref('plan');
const confirmDangerous = ref(false);
const running          = ref(false);
const runResult        = ref(null);
const runError         = ref(null);

function isTerraformKind(kind) {
  return kind === 'terraform' || kind === 'terragrunt';
}

function bundleToMarkdown(content) {
  try {
    const files = JSON.parse(content);
    return Object.entries(files)
      .map(([path, text]) => `### ${path}\n\n\`\`\`hcl\n${text}\n\`\`\`\n`)
      .join('\n');
  } catch {
    return `\`\`\`\n${content}\n\`\`\`\n`;
  }
}

const renderedContent = computed(() => {
  if (!selectedArtifact.value) return '';
  const content = isTerraformKind(selectedArtifact.value.kind)
    ? bundleToMarkdown(selectedArtifact.value.content)
    : selectedArtifact.value.content;
  return renderMarkdown(content, {}, {});
});

function kindLabel(kind) {
  if (kind === 'pdf') return 'PDF';
  if (kind === 'terraform') return 'Terraform';
  if (kind === 'terragrunt') return 'Terragrunt';
  return 'Markdown';
}

function kindBadgeClass(kind) {
  if (kind === 'pdf') return 'artifact-kind-badge--pdf';
  if (isTerraformKind(kind)) return 'artifact-kind-badge--terraform';
  return 'artifact-kind-badge--markdown';
}

const canSubmitRun = computed(() => {
  if (!selectedAgentId.value) return false;
  if (selectedAction.value !== 'plan' && !confirmDangerous.value) return false;
  return true;
});

const formattedRunResult = computed(() => {
  if (!runResult.value) return '';
  const parts = [];
  if (runResult.value.stdout) parts.push(runResult.value.stdout);
  if (runResult.value.stderr) parts.push(runResult.value.stderr);
  parts.push(`exit code: ${runResult.value.exit_code}`);
  return parts.join('\n');
});

async function openRunModal() {
  runModalOpen.value  = true;
  selectedAgentId.value  = '';
  selectedAction.value   = 'plan';
  confirmDangerous.value = false;
  runResult.value = null;
  runError.value   = null;
  try {
    availableAgents.value = await listProjectAgents(props.projectId);
  } catch {
    availableAgents.value = [];
  }
}

function closeRunModal() {
  runModalOpen.value = false;
}

async function submitRun() {
  if (!canSubmitRun.value || !selectedArtifact.value) return;
  running.value   = true;
  runResult.value = null;
  runError.value  = null;
  try {
    runResult.value = await runTerraformArtifact(
      props.projectId, selectedAgentId.value, selectedArtifact.value.id, selectedAction.value,
    );
  } catch (e) {
    runError.value = e.message || 'Run failed';
  } finally {
    running.value = false;
  }
}

function createdByLabel(createdBy) {
  return createdBy === 'assistant' ? 'Generated by the assistant' : `Created by ${createdBy}`;
}

function formatDate(iso) {
  return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

async function loadList() {
  if (!props.projectId) return;
  loading.value = true;
  try {
    artifacts.value = await listProjectArtifacts(props.projectId);
  } catch {
    artifacts.value = [];
  }
  loading.value = false;
}

async function loadArtifact(id) {
  selectedId.value       = id;
  selectedArtifact.value = null;
  contentLoading.value   = true;
  try {
    selectedArtifact.value = await getArtifact(id);
  } catch {
    selectedArtifact.value = null;
  }
  contentLoading.value = false;
}

function selectArtifact(id) {
  router.push(`/artifacts/${id}`);
}

function confirmDeleteArtifact(artifact) {
  deletingArtifact.value = artifact;
}

async function submitDeleteArtifact() {
  const artifact = deletingArtifact.value;
  if (!artifact) return;
  submitting.value = true;
  try {
    await deleteArtifact(artifact.id);
    artifacts.value = artifacts.value.filter(a => a.id !== artifact.id);
    if (selectedId.value === artifact.id) {
      selectedId.value       = null;
      selectedArtifact.value = null;
    }
    deletingArtifact.value = null;
  } catch {
  } finally {
    submitting.value = false;
  }
}

watch(() => props.projectId, () => {
  artifacts.value = [];
  if (props.projectId) loadList();
}, { immediate: true });

watch(routeArtifactId, (id) => {
  if (id) loadArtifact(id);
}, { immediate: true });
</script>

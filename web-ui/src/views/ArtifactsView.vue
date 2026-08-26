<template>
  <div class="artifacts-page">
    <div v-if="!projectId && !routeArtifactId" class="no-project-state">
      <p>Select a project to view its artifacts.</p>
    </div>

    <template v-else>
      <div class="artifacts-header">
        <div class="artifacts-header__title">
          <h2>Artifacts</h2>
          <span v-if="artifacts.length" class="artifacts-header__count">{{ artifacts.length }}</span>
        </div>
        <div v-if="projectId" class="artifacts-header__actions">
          <button class="p-button--positive is-dense" type="button" data-testid="create-artifact-btn" @click="openCreateModal">Create</button>
          <button class="p-button--positive is-dense" type="button" data-testid="upload-artifact-btn" @click="openUploadModal">Upload</button>
        </div>
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
              <span v-if="artifactRole(a.id)" class="artifact-role-badge" :data-testid="`role-badge-${a.id}`">{{ artifactRole(a.id) }}</span>
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
                  class="p-button--base run-on-agent-btn is-dense"
                  type="button"
                  @click="openRunModal"
                >
                  Run on agent
                </button>
                <a
                  class="p-button--positive artifact-download-btn is-dense"
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
          <button class="p-button--base is-dense" type="button" @click="deletingArtifact = null">Cancel</button>
          <button class="p-button--negative is-dense" type="button" :disabled="submitting" @click="submitDeleteArtifact">Delete</button>
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
          <button class="p-button--base is-dense" type="button" @click="closeRunModal">Cancel</button>
          <button
            :class="selectedAction === 'plan' ? 'p-button--positive is-dense' : 'p-button--negative is-dense'"
            type="button"
            :disabled="!canSubmitRun || running"
            @click="submitRun"
          >
            {{ running ? 'Running…' : 'Run' }}
          </button>
        </div>
      </div>
    </div>

    <div v-if="createModalOpen" class="modal" @click.self="closeCreateModal">
      <div class="modal-content modal-content--wide" data-testid="create-artifact-modal">
        <button class="modal-close" type="button" @click="closeCreateModal">✕</button>
        <h3>Create artifact</h3>
        <p class="modal-lede">Choose a type, give the artifact a title, and write its content below. Markdown and PDF artifacts are stored as text; Terraform and Terragrunt content must be a JSON bundle mapping file paths to file contents.</p>
        <div class="form-group">
          <label for="create-artifact-title">Title</label>
          <input id="create-artifact-title" v-model="createForm.title" type="text" data-testid="create-artifact-title" placeholder="Artifact title" />
        </div>
        <div class="form-group">
          <label for="create-artifact-kind">Type</label>
          <select id="create-artifact-kind" v-model="createForm.kind" data-testid="create-artifact-kind">
            <option value="markdown">Markdown</option>
            <option value="pdf">PDF (markdown source)</option>
            <option value="terraform">Terraform (JSON bundle)</option>
            <option value="terragrunt">Terragrunt (JSON bundle)</option>
            <option value="bash">Bash script</option>
          </select>
        </div>
        <div class="form-group">
          <label for="create-artifact-content">Content</label>
          <textarea id="create-artifact-content" v-model="createForm.content" rows="10" data-testid="create-artifact-content" class="artifact-modal__content" placeholder="Write the artifact content here"></textarea>
        </div>
        <div v-if="createError" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ createError }}</p>
          </div>
        </div>
        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" data-testid="cancel-create-artifact" @click="closeCreateModal">Cancel</button>
          <button class="p-button--positive is-dense" type="button" data-testid="submit-create-artifact" :disabled="createSubmitting || !canSubmitCreate" @click="submitCreate">
            {{ createSubmitting ? 'Creating…' : 'Create' }}
          </button>
        </div>
      </div>
    </div>

    <div v-if="uploadModalOpen" class="modal" @click.self="closeUploadModal">
      <div class="modal-content" data-testid="upload-artifact-modal">
        <button class="modal-close" type="button" @click="closeUploadModal">✕</button>
        <h3>Upload artifact</h3>
        <p class="modal-lede">Drag and drop a file or click to browse. Supported types: <code>.md</code>, <code>.pdf</code>, <code>.tf</code>, <code>.tf.json</code>, <code>.tg.hcl</code>, <code>.sh</code>. Terraform and Terragrunt files are uploaded as single-file bundles.</p>
        <label
          class="upload-dropzone"
          :class="{ 'upload-dropzone--active': dragActive }"
          data-testid="upload-dropzone"
          @dragover.prevent="dragActive = true"
          @dragleave.prevent="dragActive = false"
          @drop.prevent="onDrop"
        >
          <input type="file" class="upload-dropzone__input" data-testid="upload-file-input" @change="onFileChange" />
          <template v-if="!uploadFile">
            <span class="upload-dropzone__hint">Drop a file here, or click to select one</span>
          </template>
          <template v-else>
            <span class="upload-dropzone__file">{{ uploadFile.name }}</span>
            <span class="upload-dropzone__kind">{{ kindLabel(inferredKind(uploadFile.name) || '') }}</span>
          </template>
        </label>
        <div v-if="uploadFile" class="form-group">
          <label for="upload-artifact-title">Title</label>
          <input id="upload-artifact-title" v-model="uploadTitle" type="text" placeholder="Artifact title" />
        </div>
        <div v-if="uploadError" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ uploadError }}</p>
          </div>
        </div>
        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" @click="closeUploadModal">Cancel</button>
          <button class="p-button--positive is-dense" type="button" data-testid="submit-upload-artifact" :disabled="uploadSubmitting || !canSubmitUpload" @click="submitUpload">
            {{ uploadSubmitting ? 'Uploading…' : 'Upload' }}
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
  getProjectDeploymentSingle,
  createProjectArtifact,
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
const deploymentRoles  = ref({});

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
    const [list, dep] = await Promise.all([
      listProjectArtifacts(props.projectId),
      getProjectDeploymentSingle(props.projectId).catch(() => null),
    ]);
    artifacts.value = list;
    if (dep) {
      const roles = {};
      if (dep.design_doc)      roles[dep.design_doc.id] = 'design';
      if (dep.guide)           roles[dep.guide.id] = 'guide';
      if (dep.terraform_bundle) roles[dep.terraform_bundle.id] = 'bundle';
      for (const ca of (dep.context_artifacts ?? [])) roles[ca.id] = 'context';
      deploymentRoles.value = roles;
    } else {
      deploymentRoles.value = {};
    }
  } catch {
    artifacts.value = [];
    deploymentRoles.value = {};
  }
  loading.value = false;
}

function artifactRole(id) {
  return deploymentRoles.value[id] ?? null;
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

const EXTENSION_TO_KIND = [
  { exts: ['.md', '.markdown'], kind: 'markdown' },
  { exts: ['.pdf'],             kind: 'pdf' },
  { exts: ['.tf', '.tf.json'],  kind: 'terraform' },
  { exts: ['.tg.hcl', '.tg'],   kind: 'terragrunt' },
  { exts: ['.sh', '.bash'],     kind: 'bash' },
];

const SUPPORTED_EXTENSIONS = EXTENSION_TO_KIND.flatMap(e => e.exts);

function inferredKind(filename) {
  const lower = filename.toLowerCase();
  for (const { exts, kind } of EXTENSION_TO_KIND) {
    if (exts.some(ext => lower.endsWith(ext))) return kind;
  }
  return null;
}

const createModalOpen    = ref(false);
const createForm         = ref({ title: '', kind: 'markdown', content: '' });
const createSubmitting   = ref(false);
const createError        = ref(null);

const uploadModalOpen    = ref(false);
const uploadFile         = ref(null);
const uploadTitle        = ref('');
const uploadSubmitting   = ref(false);
const uploadError        = ref(null);
const dragActive         = ref(false);

const canSubmitCreate = computed(() =>
  createForm.value.title.trim().length > 0 && createForm.value.content.trim().length > 0
);

const canSubmitUpload = computed(() => !!uploadFile.value && !!inferredKind(uploadFile.value.name));

function openCreateModal() {
  createForm.value = { title: '', kind: 'markdown', content: '' };
  createError.value = null;
  createModalOpen.value = true;
}

function closeCreateModal() {
  createModalOpen.value = false;
}

async function submitCreate() {
  if (!canSubmitCreate.value) return;
  createSubmitting.value = true;
  createError.value = null;
  try {
    await createProjectArtifact(props.projectId, {
      title: createForm.value.title.trim(),
      kind: createForm.value.kind,
      content: createForm.value.content,
    });
    createModalOpen.value = false;
    await loadList();
  } catch (e) {
    createError.value = e.message || 'Failed to create artifact';
  } finally {
    createSubmitting.value = false;
  }
}

function openUploadModal() {
  uploadFile.value = null;
  uploadTitle.value = '';
  uploadError.value = null;
  dragActive.value = false;
  uploadModalOpen.value = true;
}

function closeUploadModal() {
  uploadModalOpen.value = false;
}

function onFileChange(e) {
  const input = e.target;
  const f = input?.files?.[0];
  if (f) setUploadFile(f);
}

function onDrop(e) {
  dragActive.value = false;
  const f = e.dataTransfer?.files?.[0];
  if (f) setUploadFile(f);
}

function setUploadFile(file) {
  uploadError.value = null;
  if (!inferredKind(file.name)) {
    uploadFile.value = null;
    uploadError.value = `"${file.name}" is not a supported file type. Allowed: ${SUPPORTED_EXTENSIONS.join(', ')}`;
    return;
  }
  uploadFile.value = file;
  if (!uploadTitle.value) uploadTitle.value = file.name.replace(/\.[^.]+$/, '');
}

function readFileAsText(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload  = () => resolve(String(reader.result ?? ''));
    reader.onerror = () => reject(reader.error || new Error('failed to read file'));
    reader.readAsText(file);
  });
}

async function submitUpload() {
  if (!canSubmitUpload.value) return;
  const kind = inferredKind(uploadFile.value.name);
  uploadSubmitting.value = true;
  uploadError.value = null;
  try {
    const text = await readFileAsText(uploadFile.value);
    const content = (kind === 'terraform' || kind === 'terragrunt')
      ? JSON.stringify({ [uploadFile.value.name]: text })
      : text;
    await createProjectArtifact(props.projectId, {
      title: uploadTitle.value.trim() || uploadFile.value.name,
      kind,
      content,
    });
    uploadModalOpen.value = false;
    await loadList();
  } catch (e) {
    uploadError.value = e.message || 'Failed to upload artifact';
  } finally {
    uploadSubmitting.value = false;
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

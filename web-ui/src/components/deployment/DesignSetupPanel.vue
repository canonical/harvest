<template>
  <div class="design-setup" data-testid="design-setup">
    <div class="design-setup__intro">
      <h2 class="p-heading--4">Generate a design</h2>
      <p class="u-text--muted">
        Pick a product template and select the artifacts that should inform the
        design, then generate the first version of the deployment design document.
      </p>
    </div>

    <div v-if="error" class="p-notification--negative">
      <div class="p-notification__content">
        <p class="p-notification__message">{{ error }}</p>
      </div>
    </div>

    <div class="design-setup__grid">
      <section class="design-setup__col">
        <div class="design-setup__step" data-testid="step-template">
          <span class="p-badge">1</span>
          <h3 class="p-heading--5">Product template</h3>
        </div>

        <ul class="p-list--divided design-setup__list" data-testid="template-list">
          <li v-for="t in templates" :key="t.id" class="p-list__item design-setup__item">
            <div class="p-radio design-setup__item-checkbox">
              <input
                type="radio"
                class="p-radio__input"
                :id="`template-radio-${t.id}`"
                name="design-template"
                :value="t.id"
                v-model="selectedTemplateId"
                :data-testid="`template-radio-${t.id}`"
                :disabled="busy"
              />
              <label class="p-radio__label" :for="`template-radio-${t.id}`"></label>
            </div>
            <label class="design-setup__item-title" :for="`template-radio-${t.id}`">{{ t.name }}</label>
          </li>
        </ul>
      </section>

      <section class="design-setup__col">
        <div class="design-setup__step design-setup__step--row" data-testid="step-artifacts">
          <span class="p-badge">2</span>
          <h3 class="p-heading--5">Context artifacts</h3>
          <span class="p-chip" data-testid="selection-count">{{ selectedCount }} selected</span>
          <span class="design-setup__bulk">
            <button type="button" class="p-button--positive is-dense" data-testid="create-artifact-btn" :disabled="busy" @click="openCreateModal">Create</button>
            <button type="button" class="p-button--positive is-dense" data-testid="add-artifact-btn" :disabled="busy" @click="openUploadModal">Add</button>
            <template v-if="artifacts.length">
              <button type="button" class="p-button--positive is-dense" data-testid="select-all-artifacts" :disabled="busy" @click="selectAll">Select all</button>
              <button type="button" class="p-button--positive is-dense" data-testid="clear-artifacts" :disabled="busy || !selectedArtifactIds.length" @click="clearAll">Clear</button>
            </template>
          </span>
        </div>

        <div v-if="artifactsLoading" class="design-setup__loading">
          <LoadingSpinner text="Loading artifacts…" />
        </div>
        <div v-else-if="!artifacts.length" class="design-setup__empty" data-testid="artifacts-empty">
          <p class="u-text--muted">This project has no artifacts yet.</p>
          <button type="button" class="p-button--positive is-dense" data-testid="add-artifact-btn-empty" @click="openUploadModal">Add an artifact</button>
        </div>
        <ul v-else class="p-list--divided design-setup__list">
          <li v-for="a in artifacts" :key="a.id" class="p-list__item design-setup__item">
            <div class="p-checkbox design-setup__item-checkbox">
              <input
                type="checkbox"
                class="p-checkbox__input"
                :id="`artifact-checkbox-${a.id}`"
                :value="a.id"
                v-model="selectedArtifactIds"
                :data-testid="`artifact-checkbox-${a.id}`"
                :disabled="busy"
              />
              <label class="p-checkbox__label" :for="`artifact-checkbox-${a.id}`"></label>
            </div>
            <label class="design-setup__item-title" :for="`artifact-checkbox-${a.id}`">{{ a.title }}</label>
            <span class="artifact-kind-badge" :class="kindBadgeClass(a.kind)">{{ kindLabel(a.kind) }}</span>
            <span class="p-text--small u-text--muted design-setup__item-date">{{ formatDate(a.created_at) }}</span>
          </li>
        </ul>
      </section>
    </div>

    <div class="design-setup__footer">
      <p class="u-text--muted" data-testid="generation-summary">{{ generationSummary }}</p>
      <div class="design-setup__actions">
        <button
          class="p-button--positive"
          type="button"
          data-testid="generate-design-btn"
          :disabled="busy || !canGenerate"
          @click="generate"
        >{{ busy ? 'Generating…' : 'Generate design' }}</button>
        <BusyStatus v-if="busy" text="Generating design…" />
      </div>
    </div>

    <div v-if="createModalOpen" class="modal" @click.self="closeCreateModal">
      <div class="modal-content modal-content--wide" data-testid="create-artifact-modal">
        <button class="modal-close" type="button" @click="closeCreateModal">✕</button>
        <h3>Create artifact</h3>
        <p class="modal-lede">Choose a type, give the artifact a title, and write its content below. Markdown and PDF artifacts are stored as text; Terraform and Terragrunt content must be a JSON bundle mapping file paths to file contents.</p>
        <div class="form-group">
          <label for="design-create-artifact-title">Title</label>
          <input id="design-create-artifact-title" v-model="createForm.title" type="text" data-testid="create-artifact-title" placeholder="Artifact title" />
        </div>
        <div class="form-group">
          <label for="design-create-artifact-kind">Type</label>
          <select id="design-create-artifact-kind" v-model="createForm.kind" data-testid="create-artifact-kind">
            <option value="markdown">Markdown</option>
            <option value="pdf">PDF (markdown source)</option>
            <option value="terraform">Terraform (JSON bundle)</option>
            <option value="terragrunt">Terragrunt (JSON bundle)</option>
            <option value="bash">Bash script</option>
          </select>
        </div>
        <div class="form-group">
          <label for="design-create-artifact-content">Content</label>
          <textarea id="design-create-artifact-content" v-model="createForm.content" rows="10" data-testid="create-artifact-content" class="artifact-modal__content" placeholder="Write the artifact content here"></textarea>
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
        <h3>Add artifact</h3>
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
          <label for="design-upload-artifact-title">Title</label>
          <input id="design-upload-artifact-title" v-model="uploadTitle" type="text" placeholder="Artifact title" />
        </div>
        <div v-if="uploadError" class="p-notification--negative">
          <div class="p-notification__content">
            <p class="p-notification__message">{{ uploadError }}</p>
          </div>
        </div>
        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" @click="closeUploadModal">Cancel</button>
          <button class="p-button--positive is-dense" type="button" data-testid="submit-upload-artifact" :disabled="uploadSubmitting || !canSubmitUpload" @click="submitUpload">
            {{ uploadSubmitting ? 'Adding…' : 'Add' }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import { listProjectArtifacts, listTemplates, createProjectArtifact } from '../../lib/api.js';
import BusyStatus from './BusyStatus.vue';
import LoadingSpinner from './LoadingSpinner.vue';

const props = defineProps({
  projectId:    { type: String, required: true },
  deploymentId: { type: String, required: true },
  groupId:      { type: String, default: null },
});
const emit = defineEmits(['generate']);

const artifacts           = ref([]);
const templates           = ref([]);
const artifactsLoading    = ref(false);
const selectedTemplateId  = ref('');
const selectedArtifactIds = ref([]);
const busy                = ref(false);
const error               = ref(null);

const selectedTemplate = computed(() => templates.value.find(t => t.id === selectedTemplateId.value) ?? null);
const selectedCount    = computed(() => selectedArtifactIds.value.length);
const canGenerate      = computed(() => !!selectedTemplateId.value && selectedCount.value > 0);

const generationSummary = computed(() => {
  if (!canGenerate.value) {
    return 'Select a product template and at least one context artifact to generate a design.';
  }
  const n = selectedCount.value;
  const artifactPart = n === 1 ? '1 artifact' : `${n} artifacts`;
  return `Generating with the ${selectedTemplate.value.name} template and ${artifactPart}.`;
});

function kindLabel(kind) {
  if (kind === 'pdf') return 'PDF';
  if (kind === 'terraform') return 'Terraform';
  if (kind === 'terragrunt') return 'Terragrunt';
  if (kind === 'bash') return 'Bash';
  return 'Markdown';
}

function kindBadgeClass(kind) {
  if (kind === 'pdf') return 'artifact-kind-badge--pdf';
  if (kind === 'terraform' || kind === 'terragrunt') return 'artifact-kind-badge--terraform';
  if (kind === 'bash') return 'artifact-kind-badge--bash';
  return 'artifact-kind-badge--markdown';
}

function formatDate(iso) {
  return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

function selectAll() {
  selectedArtifactIds.value = artifacts.value.map(a => a.id);
}

function clearAll() {
  selectedArtifactIds.value = [];
}

async function load() {
  artifactsLoading.value = true;
  error.value = null;
  try {
    const [listResult, tplsResult] = await Promise.allSettled([
      listProjectArtifacts(props.projectId),
      listTemplates(),
    ]);
    artifacts.value = listResult.status === 'fulfilled' ? listResult.value : [];
    templates.value = tplsResult.status === 'fulfilled' ? tplsResult.value : [];
    if (listResult.status === 'rejected' && tplsResult.status === 'rejected') {
      error.value = listResult.reason?.message || 'Failed to load context';
    } else if (listResult.status === 'rejected') {
      error.value = listResult.reason?.message || 'Failed to load artifacts';
    } else if (tplsResult.status === 'rejected') {
      error.value = tplsResult.reason?.message || 'Failed to load templates';
    }
  } finally {
    artifactsLoading.value = false;
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

const createModalOpen  = ref(false);
const createForm       = ref({ title: '', kind: 'markdown', content: '' });
const createSubmitting = ref(false);
const createError      = ref(null);

const uploadModalOpen  = ref(false);
const uploadFile       = ref(null);
const uploadTitle      = ref('');
const uploadSubmitting = ref(false);
const uploadError      = ref(null);
const dragActive       = ref(false);

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
    const created = await createProjectArtifact(props.projectId, {
      title: createForm.value.title.trim(),
      kind: createForm.value.kind,
      content: createForm.value.content,
    });
    createModalOpen.value = false;
    await load();
    if (created?.id) selectedArtifactIds.value = [...selectedArtifactIds.value, created.id];
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
    const created = await createProjectArtifact(props.projectId, {
      title: uploadTitle.value.trim() || uploadFile.value.name,
      kind,
      content,
    });
    uploadModalOpen.value = false;
    await load();
    if (created?.id) selectedArtifactIds.value = [...selectedArtifactIds.value, created.id];
  } catch (e) {
    uploadError.value = e.message || 'Failed to add artifact';
  } finally {
    uploadSubmitting.value = false;
  }
}

function generate() {
  busy.value = true;
  error.value = null;
  emit('generate', {
    artifact_ids: [...selectedArtifactIds.value],
    product_template_id: selectedTemplateId.value || null,
  });
  busy.value = false;
}

watch(() => [props.projectId, props.deploymentId], () => load(), { immediate: true });
</script>

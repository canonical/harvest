<template>
  <div class="artifact-editor" data-testid="artifact-editor">
    <div v-if="!artifactId" class="artifact-editor__empty" data-testid="artifact-editor-empty">
      <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
      <p>Select an artifact to view and edit it.</p>
    </div>

    <div v-else-if="loading" class="artifact-editor__loading" data-testid="artifact-editor-loading">
      <LoadingSpinner text="Loading artifact…" />
    </div>

    <div v-else-if="artifact" class="artifact-editor__content">
      <div class="artifact-editor__header">
        <div class="artifact-editor__meta">
          <h3 class="artifact-editor__title">{{ artifact.title }}</h3>
          <span class="artifact-kind-badge" :class="kindBadgeClass(artifact.kind)">{{ kindLabel(artifact.kind) }}</span>
          <span v-if="dirty" class="artifact-editor__diff-badge" data-testid="diff-badge">
            <span class="artifact-editor__diff-added">+{{ diffStats.added }}</span>
            <span class="artifact-editor__diff-removed">−{{ diffStats.removed }}</span>
          </span>
        </div>
        <div class="artifact-editor__actions">
          <button
            class="p-button--positive is-dense"
            type="button"
            data-testid="save-artifact-btn"
            :disabled="!dirty || saving"
            @click="save"
          >{{ saving ? 'Saving…' : 'Save' }}</button>
        </div>
      </div>

      <div v-if="bashPair" class="artifact-editor__tabs" data-testid="script-tabs">
        <button
          type="button"
          class="artifact-editor__tab"
          :class="{ 'artifact-editor__tab--active': scriptTab === 'deploy' }"
          data-testid="script-tab-deploy"
          @click="emit('script-tab-change', 'deploy')"
        >Deploy</button>
        <button
          type="button"
          class="artifact-editor__tab"
          :class="{ 'artifact-editor__tab--active': scriptTab === 'destroy' }"
          data-testid="script-tab-destroy"
          @click="emit('script-tab-change', 'destroy')"
        >Destroy</button>
      </div>

      <div v-if="error" class="p-notification--negative artifact-editor__error" data-testid="artifact-editor-error">
        <div class="p-notification__content">
          <p class="p-notification__message">{{ error }}</p>
        </div>
      </div>

      <div v-if="isBundle" class="artifact-editor__tabs" data-testid="artifact-editor-tabs">
        <button
          v-for="path in filePaths"
          :key="path"
          class="artifact-editor__tab"
          :class="{ 'artifact-editor__tab--active': activeTab === path }"
          :data-testid="`artifact-tab-${path}`"
          @click="switchTab(path)"
        >{{ path }}</button>
      </div>
      <div ref="containerRef" class="artifact-editor__container" data-testid="artifact-editor-container" />
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from 'vue';
import { getArtifact, updateArtifact } from '../../lib/api.js';
import { isDarkTheme, onThemeChange } from '../../lib/theme.js';
import LoadingSpinner from './LoadingSpinner.vue';

const props = defineProps({
  projectId:    { type: String, required: true },
  deploymentId: { type: String, required: true },
  artifactId:   { type: String, default: null },
  bashPair:     { type: Object, default: null },
  scriptTab:    { type: String, default: 'deploy' },
});
const emit = defineEmits(['saved', 'script-tab-change']);

const containerRef = ref(null);
const artifact = ref(null);
const loading  = ref(false);
const dirty    = ref(false);
const saving   = ref(false);
const error    = ref(null);

let editor      = null;
let monacoApi    = null;
let originalContent = '';

const filePaths  = ref([]);
const fileContents = {};
const originalFiles = {};
const activeTab  = ref('');

const isBundle = ref(false);

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

function languageForKind(kind) {
  if (kind === 'terraform' || kind === 'terragrunt') return 'json';
  if (kind === 'bash') return 'shell';
  return 'markdown';
}

function languageForFile(path) {
  if (path.endsWith('.tf')) return 'hcl';
  if (path.endsWith('.hcl')) return 'hcl';
  if (path.endsWith('.sh') || path.endsWith('.bash')) return 'shell';
  if (path.endsWith('.json')) return 'json';
  if (path.endsWith('.yaml') || path.endsWith('.yml')) return 'yaml';
  if (path.endsWith('.md')) return 'markdown';
  return 'plaintext';
}

function parseBundle(content) {
  try {
    const obj = JSON.parse(content);
    if (obj && typeof obj === 'object' && !Array.isArray(obj)) {
      return obj;
    }
  } catch {}
  return null;
}

function serializeBundle(obj) {
  return JSON.stringify(obj, null, 2);
}

function lcs(a, b) {
  const n = a.length;
  const m = b.length;
  if (n === 0 || m === 0) return [];
  const dp = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = 1; i <= n; i++) {
    for (let j = 1; j <= m; j++) {
      dp[i][j] = a[i - 1] === b[j - 1]
        ? dp[i - 1][j - 1] + 1
        : Math.max(dp[i - 1][j], dp[i][j - 1]);
    }
  }
  const result = [];
  let i = n, j = m;
  while (i > 0 && j > 0) {
    if (a[i - 1] === b[j - 1]) {
      result.unshift({ type: 'same', line: a[i - 1] });
      i--; j--;
    } else if (dp[i - 1][j] >= dp[i][j - 1]) {
      result.unshift({ type: 'removed', line: a[i - 1] });
      i--;
    } else {
      result.unshift({ type: 'added', line: b[j - 1] });
      j--;
    }
  }
  while (i > 0) { result.unshift({ type: 'removed', line: a[i - 1] }); i--; }
  while (j > 0) { result.unshift({ type: 'added', line: b[j - 1] }); j--; }
  return result;
}

function computeLineDiff(original, current) {
  const origLines = original.split('\n');
  const currLines = current.split('\n');
  const diff = lcs(origLines, currLines);
  let added = 0;
  let removed = 0;
  for (const d of diff) {
    if (d.type === 'added') added++;
    else if (d.type === 'removed') removed++;
  }
  return { added, removed };
}

const diffStats = computed(() => {
  if (!dirty.value) return { added: 0, removed: 0 };
  if (isBundle.value) {
    let added = 0;
    let removed = 0;
    for (const path of filePaths.value) {
      const orig = originalFiles[path] ?? '';
      const curr = fileContents[path] ?? '';
      if (orig !== curr) {
        const stats = computeLineDiff(orig, curr);
        added += stats.added;
        removed += stats.removed;
      }
    }
    return { added, removed };
  }
  const curr = editor ? editor.getValue() : originalContent;
  return computeLineDiff(originalContent, curr);
});

async function loadArtifact() {
  if (!props.artifactId) {
    artifact.value = null;
    dirty.value = false;
    return;
  }
  loading.value = true;
  error.value = null;
  dirty.value = false;
  try {
    const a = await getArtifact(props.artifactId);
    artifact.value = a;
    originalContent = a.content ?? '';
    const bundle = parseBundle(originalContent);
    if (a.kind === 'terraform' || a.kind === 'terragrunt') {
      if (bundle) {
        isBundle.value = true;
        filePaths.value = Object.keys(bundle).sort();
        for (const path of filePaths.value) {
          fileContents[path] = bundle[path] ?? '';
          originalFiles[path] = bundle[path] ?? '';
        }
        activeTab.value = filePaths.value[0] ?? '';
      } else {
        isBundle.value = false;
      }
    } else {
      isBundle.value = false;
    }
    loading.value = false;
    await nextTick();
    mountEditor();
  } catch (e) {
    artifact.value = null;
    error.value = e.message || 'Failed to load artifact';
    loading.value = false;
  }
}

function getCurrentContent() {
  if (isBundle.value) {
    if (editor && activeTab.value) {
      fileContents[activeTab.value] = editor.getValue();
    }
    return serializeBundle(fileContents);
  }
  return editor ? editor.getValue() : originalContent;
}

function checkDirty() {
  if (isBundle.value) {
    if (editor && activeTab.value) {
      fileContents[activeTab.value] = editor.getValue();
    }
    for (const path of filePaths.value) {
      if ((fileContents[path] ?? '') !== (originalFiles[path] ?? '')) {
        dirty.value = true;
        return;
      }
    }
    dirty.value = false;
  } else if (editor) {
    dirty.value = editor.getValue() !== originalContent;
  }
}

function disposeEditor() {
  if (editor) {
    editor.dispose();
    editor = null;
  }
}

async function mountEditor() {
  disposeEditor();
  if (!containerRef.value || !artifact.value) return;
  if (!monacoApi) {
    monacoApi = await import('monaco-editor');
  }
  const value = isBundle.value
    ? (fileContents[activeTab.value] ?? '')
    : originalContent;
  const language = isBundle.value
    ? languageForFile(activeTab.value)
    : languageForKind(artifact.value.kind);
  editor = monacoApi.editor.create(containerRef.value, {
    value,
    language,
    theme: isDarkTheme() ? 'vs-dark' : 'vs',
    automaticLayout: true,
    minimap: { enabled: false },
    fontSize: 13,
    lineNumbers: 'on',
    wordWrap: 'on',
    scrollBeyondLastLine: false,
    readOnly: false,
  });
  editor.onDidChangeModelContent(() => {
    checkDirty();
  });
}

function switchTab(path) {
  if (editor && activeTab.value) {
    fileContents[activeTab.value] = editor.getValue();
  }
  activeTab.value = path;
  if (editor) {
    editor.setValue(fileContents[path] ?? '');
    const lang = languageForFile(path);
    const model = editor.getModel();
    if (model) monacoApi.editor.setModelLanguage(model, lang);
    checkDirty();
  }
}

async function save() {
  if (saving.value) return;
  const newContent = getCurrentContent();
  if (!dirty.value) return;
  saving.value = true;
  error.value = null;
  try {
    await updateArtifact(props.artifactId, {
      title: artifact.value.title,
      kind: artifact.value.kind,
      content: newContent,
    });
    originalContent = newContent;
    if (isBundle.value) {
      if (editor && activeTab.value) {
        fileContents[activeTab.value] = editor.getValue();
      }
      for (const path of filePaths.value) {
        originalFiles[path] = fileContents[path] ?? '';
      }
    }
    dirty.value = false;
    emit('saved');
  } catch (e) {
    error.value = e.message || 'Failed to save';
  } finally {
    saving.value = false;
  }
}

function handleKeydown(e) {
  if (e.ctrlKey && (e.key === 's' || e.key === 'S')) {
    e.preventDefault();
    if (!dirty.value) return;
    save();
  }
}

watch(() => props.artifactId, () => loadArtifact());

const unsubscribeTheme = onThemeChange(() => {
  monacoApi?.editor.setTheme(isDarkTheme() ? 'vs-dark' : 'vs');
});

onMounted(() => {
  loadArtifact();
  window.addEventListener('keydown', handleKeydown);
});

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleKeydown);
  disposeEditor();
  unsubscribeTheme();
});

defineExpose({ handleKeydown });
</script>

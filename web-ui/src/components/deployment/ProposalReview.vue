<template>
  <div class="proposal-review" data-testid="proposal-review">
    <div class="proposal-review__header">
      <div class="proposal-review__title-group">
        <h3 class="proposal-review__title">Proposed changes</h3>
        <span v-if="changedCount > 0" class="proposal-review__count">{{ changedCount }} file{{ changedCount > 1 ? 's' : '' }} changed</span>
      </div>
      <div class="proposal-review__actions">
        <button
          class="p-button--positive is-dense"
          type="button"
          data-testid="apply-proposal-btn"
          :disabled="applying"
          @click="apply"
        >{{ applying ? 'Applying…' : 'Apply' }}</button>
        <button
          class="p-button--base is-dense"
          type="button"
          data-testid="modify-proposal-btn"
          :disabled="applying"
          @click="$emit('modify')"
        >Modify</button>
        <button
          class="p-button--negative is-dense"
          type="button"
          data-testid="discard-proposal-btn"
          :disabled="applying"
          @click="$emit('discard')"
        >Discard</button>
      </div>
    </div>

    <div v-if="explanation" class="proposal-review__explanation" data-testid="proposal-explanation" v-html="renderedExplanation" />

    <div class="proposal-review__body">
      <div class="proposal-review__tree" data-testid="proposal-tree">
        <template v-for="group in treeGroups" :key="group.label">
          <div class="proposal-review__tree-group">
            <button
              type="button"
              class="proposal-review__tree-header"
              @click="toggleGroup(group.label)"
            >
              <span class="proposal-review__tree-chevron" :class="{ 'proposal-review__tree-chevron--open': group.open }">▶</span>
              <span class="proposal-review__tree-label">{{ group.label }}</span>
              <span v-if="group.changedCount > 0" class="proposal-review__tree-badge">{{ group.changedCount }}</span>
            </button>
            <ul v-if="group.open" class="proposal-review__tree-list">
              <li
                v-for="file in group.files"
                :key="file.path"
                class="proposal-review__tree-item"
                :class="{
                  'proposal-review__tree-item--active': activePath === file.path,
                  'proposal-review__tree-item--new': file.isNew,
                  'proposal-review__tree-item--changed': file.isChanged,
                  'proposal-review__tree-item--unchanged': !file.isChanged && !file.isNew,
                }"
                :data-testid="`proposal-file-${file.path}`"
                @click="selectFile(file.path)"
              >
                <span v-if="file.isNew" class="proposal-review__tree-indicator proposal-review__tree-indicator--new">+</span>
                <span v-else-if="file.isChanged" class="proposal-review__tree-indicator proposal-review__tree-indicator--changed">●</span>
                <span v-else class="proposal-review__tree-indicator" />
                <span class="proposal-review__tree-filename">{{ file.path }}</span>
              </li>
            </ul>
          </div>
        </template>
      </div>

      <div ref="containerRef" class="proposal-review__diff" data-testid="proposal-diff-editor" />
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from 'vue';
import { getArtifact } from '../../lib/api.js';
import { isDarkTheme, onThemeChange } from '../../lib/theme.js';
import { renderMarkdown } from '../../lib/markdown.js';

const props = defineProps({
  projectId:       { type: String, required: true },
  deploymentId:    { type: String, required: true },
  proposedFiles:   { type: Object, required: true },
  originalFiles:   { type: Object, default: () => ({}) },
  executionPlan:   { type: Object, default: () => ({ deploy_steps: [], destroy_steps: [] }) },
});

const emit = defineEmits(['apply', 'discard', 'modify']);

const containerRef = ref(null);
const activePath = ref('');
const applying = ref(false);
const openGroups = ref({});

let diffEditor = null;
let monacoApi = null;
const diffOrigModels = {};
const diffModModels = {};
const resolvedOriginals = {};

const renderedExplanation = computed(() => {
  return '';
});

const changedCount = computed(() => {
  let count = 0;
  for (const path of Object.keys(props.proposedFiles)) {
    const orig = resolvedOriginals[path] ?? '';
    const proposed = props.proposedFiles[path] ?? '';
    if (orig !== proposed) count++;
  }
  return count;
});

function isTerraformFile(path) {
  const lower = path.toLowerCase();
  return lower.endsWith('.tf') || lower.endsWith('.hcl') || lower.endsWith('.json') || lower.endsWith('.yaml') || lower.endsWith('.yml');
}

function isBashFile(path) {
  const lower = path.toLowerCase();
  return lower.endsWith('.sh') || lower.endsWith('.bash') || lower.startsWith('deploy-') || lower.startsWith('destroy-');
}

function fileStatus(path) {
  const orig = resolvedOriginals[path] ?? '';
  const proposed = props.proposedFiles[path] ?? '';
  if (!orig && proposed) return 'new';
  if (orig !== proposed) return 'changed';
  return 'unchanged';
}

const treeGroups = computed(() => {
  const groups = [];
  const bundleFiles = [];
  const bashFiles = [];
  const otherFiles = [];

  for (const path of Object.keys(props.proposedFiles)) {
    const status = fileStatus(path);
    const file = { path, isNew: status === 'new', isChanged: status === 'changed' };
    if (isTerraformFile(path)) {
      bundleFiles.push(file);
    } else if (isBashFile(path)) {
      bashFiles.push(file);
    } else {
      otherFiles.push(file);
    }
  }

  const sortFn = (a, b) => {
    if (a.isNew !== b.isNew) return a.isNew ? -1 : 1;
    if (a.isChanged !== b.isChanged) return a.isChanged ? -1 : 1;
    return a.path.localeCompare(b.path);
  };

  bundleFiles.sort(sortFn);
  bashFiles.sort(sortFn);
  otherFiles.sort(sortFn);

  if (bundleFiles.length > 0) {
    const label = 'Terraform bundle';
    groups.push({
      label,
      files: bundleFiles,
      open: openGroups.value[label] !== false,
      changedCount: bundleFiles.filter(f => f.isChanged || f.isNew).length,
    });
  }
  if (bashFiles.length > 0) {
    const label = 'Bash scripts';
    groups.push({
      label,
      files: bashFiles,
      open: openGroups.value[label] !== false,
      changedCount: bashFiles.filter(f => f.isChanged || f.isNew).length,
    });
  }
  if (otherFiles.length > 0) {
    const label = 'Other files';
    groups.push({
      label,
      files: otherFiles,
      open: openGroups.value[label] !== false,
      changedCount: otherFiles.filter(f => f.isChanged || f.isNew).length,
    });
  }

  return groups;
});

function toggleGroup(label) {
  openGroups.value[label] = openGroups.value[label] === false;
}

function languageForFile(path) {
  if (path.endsWith('.tf') || path.endsWith('.hcl')) return 'hcl';
  if (path.endsWith('.sh') || path.endsWith('.bash')) return 'shell';
  if (path.endsWith('.json')) return 'json';
  if (path.endsWith('.yaml') || path.endsWith('.yml')) return 'yaml';
  if (path.endsWith('.md')) return 'markdown';
  return 'plaintext';
}

async function resolveOriginals() {
  for (const path of Object.keys(props.proposedFiles)) {
    if (props.originalFiles[path] !== undefined) {
      resolvedOriginals[path] = props.originalFiles[path] ?? '';
      continue;
    }

    if (isTerraformFile(path)) {
      resolvedOriginals[path] = '';
      continue;
    }

    if (isBashFile(path)) {
      const matched = findOriginalByTitle(path);
      if (matched) {
        resolvedOriginals[path] = matched;
      } else {
        resolvedOriginals[path] = '';
      }
      continue;
    }

    resolvedOriginals[path] = '';
  }
}

function findOriginalByTitle(title) {
  const steps = [
    ...(props.executionPlan.deploy_steps ?? []),
    ...(props.executionPlan.destroy_steps ?? []),
  ];
  for (const step of steps) {
    if (step.artifact?.title === title) {
      return step.artifact?._content ?? null;
    }
  }
  return null;
}

async function fetchOriginals() {
  const steps = [
    ...(props.executionPlan.deploy_steps ?? []),
    ...(props.executionPlan.destroy_steps ?? []),
  ];
  for (const path of Object.keys(props.proposedFiles)) {
    if (resolvedOriginals[path] !== undefined && resolvedOriginals[path] !== '') continue;
    if (!isBashFile(path)) continue;

    for (const step of steps) {
      if (step.artifact?.title === path && step.artifact?.id) {
        try {
          const artifact = await getArtifact(step.artifact.id);
          resolvedOriginals[path] = artifact.content ?? '';
        } catch {
          resolvedOriginals[path] = '';
        }
        break;
      }
    }
  }
}

async function mountDiffEditor() {
  if (!containerRef.value) return;
  if (!monacoApi) {
    monacoApi = await import('monaco-editor');
  }
  if (diffEditor) {
    diffEditor.dispose();
    diffEditor = null;
  }
  for (const k of Object.keys(diffOrigModels)) { diffOrigModels[k]?.dispose(); delete diffOrigModels[k]; }
  for (const k of Object.keys(diffModModels)) { diffModModels[k]?.dispose(); delete diffModModels[k]; }

  diffEditor = monacoApi.editor.createDiffEditor(containerRef.value, {
    theme: isDarkTheme() ? 'vs-dark' : 'vs',
    automaticLayout: true,
    minimap: { enabled: false },
    fontSize: 13,
    lineNumbers: 'on',
    wordWrap: 'on',
    scrollBeyondLastLine: false,
    renderSideBySide: true,
    originalEditable: false,
    readOnly: true,
  });

  for (const path of Object.keys(props.proposedFiles)) {
    const lang = languageForFile(path);
    diffOrigModels[path] = monacoApi.editor.createModel(resolvedOriginals[path] ?? '', lang);
    diffModModels[path] = monacoApi.editor.createModel(props.proposedFiles[path] ?? '', lang);
  }

  const allPaths = Object.keys(props.proposedFiles).sort();
  const firstChanged = allPaths.find(p => fileStatus(p) !== 'unchanged') ?? allPaths[0];
  activePath.value = firstChanged ?? '';
  applyDiffModels();
}

function applyDiffModels() {
  if (!diffEditor) return;
  const path = activePath.value;
  diffEditor.setModel({
    original: diffOrigModels[path],
    modified: diffModModels[path],
  });
}

function selectFile(path) {
  activePath.value = path;
  applyDiffModels();
}

async function apply() {
  if (applying.value) return;
  applying.value = true;
  try {
    const filesToApply = {};
    for (const path of Object.keys(props.proposedFiles)) {
      if (diffModModels[path]) {
        filesToApply[path] = diffModModels[path].getValue();
      } else {
        filesToApply[path] = props.proposedFiles[path];
      }
    }
    emit('apply', filesToApply);
  } finally {
    applying.value = false;
  }
}

function disposeDiffEditor() {
  if (diffEditor) {
    diffEditor.dispose();
    diffEditor = null;
  }
  for (const k of Object.keys(diffOrigModels)) { diffOrigModels[k]?.dispose(); delete diffOrigModels[k]; }
  for (const k of Object.keys(diffModModels)) { diffModModels[k]?.dispose(); delete diffModModels[k]; }
}

const unsubscribeTheme = onThemeChange(() => {
  monacoApi?.editor.setTheme(isDarkTheme() ? 'vs-dark' : 'vs');
});

onMounted(async () => {
  await resolveOriginals();
  await fetchOriginals();
  await nextTick();
  await mountDiffEditor();
});

onBeforeUnmount(() => {
  disposeDiffEditor();
  unsubscribeTheme();
});
</script>

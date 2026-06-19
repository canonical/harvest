<template>
  <div class="docs-page">
    <div class="doc-toolbar">
      <div class="doc-selector">
        <label for="doc-repo-select">Repository</label>
        <select id="doc-repo-select" v-model="selectedRepo" @change="onRepoChange">
          <option value="">Choose a repository…</option>
          <option v-for="r in repos" :key="r.name" :value="r.name">{{ r.name }}</option>
        </select>
      </div>
      <div class="doc-selector">
        <label for="doc-version-select">Version</label>
        <select
          id="doc-version-select"
          v-model="selectedVersion"
          :disabled="!selectedRepo"
          @change="onVersionChange"
        >
          <option value="">Choose a version…</option>
          <option v-for="v in currentVersions" :key="v" :value="v">{{ v }}</option>
        </select>
      </div>
    </div>

    <div class="doc-layout">
      <nav class="doc-tree">
        <template v-if="!selectedRepo || !selectedVersion">
          <p class="doc-tree__hint">Select a repository and version to browse documentation.</p>
        </template>
        <template v-else-if="indexLoading">
          <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
        </template>
        <template v-else-if="indexError">
          <p class="doc-tree__hint doc-tree__hint--error">{{ indexError }}</p>
        </template>
        <template v-else-if="!docIndex">
          <p class="doc-tree__hint">No documentation generated yet.</p>
        </template>
        <template v-else>
          <div v-for="section in SECTION_ORDER" :key="section" class="doc-tree__section">
            <button
              class="doc-tree__section-btn"
              :class="{ 'doc-tree__section-btn--expanded': expandedSections.has(section) }"
              @click="toggleSection(section)"
            >
              <span>{{ expandedSections.has(section) ? '▾' : '▸' }}</span>
              {{ SECTION_LABELS[section] }}
              <span class="doc-tree__section-count">{{ (docIndex.sections[section] ?? []).length }}</span>
            </button>
            <ul v-if="expandedSections.has(section)" class="doc-tree__pages">
              <li
                v-for="page in (docIndex.sections[section] ?? [])"
                :key="page.filename"
                class="doc-tree__page"
                :class="{ 'doc-tree__page--active': selectedSection === section && selectedFile === page.filename }"
              >
                <button
                  class="doc-tree__page-btn"
                  @click="loadPage(section, page.filename)"
                >{{ page.title }}</button>
              </li>
              <li v-if="!(docIndex.sections[section] ?? []).length" class="doc-tree__empty">No pages</li>
            </ul>
          </div>
        </template>
      </nav>

      <main class="doc-content doc-content-area">
        <div v-if="docIndex && !selectedFile" class="doc-welcome">
          <h2>{{ docIndex.repo }} {{ docIndex.version }}</h2>
          <p>{{ totalPages }} page{{ totalPages !== 1 ? 's' : '' }} of documentation available. Select a page from the sidebar to read it.</p>
        </div>
        <div v-else-if="!selectedFile" class="doc-welcome">
          <p>Select a repository and version<br>to browse its documentation</p>
        </div>
        <div v-else-if="pageLoading" class="doc-content-loading">
          <span class="loading-dots"><span>.</span><span>.</span><span>.</span></span>
        </div>
        <div v-else-if="pageError" class="doc-content-error">
          <strong>Error:</strong> {{ pageError }}
        </div>
        <article v-else-if="pageContent" class="doc-article">
          <div class="doc-article__body" v-html="renderedPage" />
        </article>
      </main>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue';
import { renderMarkdown } from '../lib/markdown.js';
import { fetchDocIndex, fetchDocPage, fetchRepositories } from '../lib/api.js';

const SECTION_ORDER  = ['tutorials', 'how-to-guides', 'explanations', 'reference'];
const SECTION_LABELS = {
  tutorials:        'Tutorials',
  'how-to-guides':  'How-to Guides',
  explanations:     'Explanations',
  reference:        'Reference',
};

const repos = ref([]);

const selectedRepo    = ref('');
const selectedVersion = ref('');
const docIndex        = ref(null);
const indexLoading    = ref(false);
const indexError      = ref(null);
const expandedSections = ref(new Set(['tutorials']));

const selectedSection = ref(null);
const selectedFile    = ref(null);
const pageContent     = ref(null);
const pageLoading     = ref(false);
const pageError       = ref(null);

const currentVersions = computed(() => {
  const repo = repos.value.find(r => r.name === selectedRepo.value);
  if (!repo) return [];
  return [...repo.versions].sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
});

const totalPages = computed(() => {
  if (!docIndex.value) return 0;
  return SECTION_ORDER.reduce((n, s) => n + (docIndex.value.sections[s]?.length ?? 0), 0);
});

const renderedPage = computed(() =>
  pageContent.value ? renderMarkdown(pageContent.value, {}, {}) : ''
);

function onRepoChange() {
  selectedVersion.value = '';
  docIndex.value        = null;
  indexError.value      = null;
  selectedSection.value = null;
  selectedFile.value    = null;
  pageContent.value     = null;
}

async function onVersionChange() {
  if (!selectedVersion.value) return;
  docIndex.value    = null;
  indexError.value  = null;
  indexLoading.value = true;
  selectedSection.value = null;
  selectedFile.value    = null;
  pageContent.value     = null;
  try {
    const idx = await fetchDocIndex(selectedRepo.value, selectedVersion.value);
    if (idx) {
      docIndex.value = idx;
    } else {
      indexError.value = 'No documentation generated yet for this version.';
    }
  } catch (e) {
    indexError.value = e.message;
  }
  indexLoading.value = false;
}

function toggleSection(section) {
  const set = new Set(expandedSections.value);
  if (set.has(section)) { set.delete(section); } else { set.add(section); }
  expandedSections.value = set;
}

async function loadPage(section, filename) {
  selectedSection.value = section;
  selectedFile.value    = filename;
  pageContent.value     = null;
  pageError.value       = null;
  pageLoading.value     = true;
  try {
    const content = await fetchDocPage(selectedRepo.value, selectedVersion.value, section, filename);
    if (content !== null) {
      pageContent.value = content;
    } else {
      pageError.value = 'Page not found.';
    }
  } catch (e) {
    pageError.value = e.message;
  }
  pageLoading.value = false;
}

onMounted(async () => {
  try { repos.value = await fetchRepositories(); } catch {}
});
</script>

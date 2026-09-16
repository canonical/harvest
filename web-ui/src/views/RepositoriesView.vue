<template>
  <div class="repositories-page">
    <div class="repositories-header">
      <div class="repositories-header__title">
        <h2>Code repositories</h2>
        <span v-if="repos.length" class="repositories-header__count">{{ repos.length }}</span>
      </div>
      <div class="repositories-header__actions">
        <button class="p-button--positive is-dense" type="button" @click="openAddModal">+ Add repository</button>
      </div>
    </div>

    <div class="repositories-layout">
      <div class="repositories-list">
        <div class="repo-list-search-bar">
          <svg class="repo-list-search-bar__icon" xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>
          <input
            v-model="repoSearch"
            type="search"
            placeholder="Search repositories…"
            autocomplete="off"
          />
        </div>
        <div v-if="listLoading" class="repositories-list-loading">
          <LoadingSpinner text="Loading…" />
        </div>
        <p v-else-if="!filteredRepos.length" class="repositories-list-empty">
          {{ repos.length ? 'No repositories match your search.' : 'No repositories yet. Click "Add repository" to ingest one.' }}
        </p>
        <button
          v-for="r in filteredRepos"
          :key="r.name"
          class="repo-list-item"
          :class="{ 'repo-list-item--active': r.name === selectedRepo }"
          type="button"
          @click="selectRepo(r.name)"
        >
          <span class="repo-list-item__name">{{ r.name }}</span>
          <span class="repo-list-item__meta">
            <span v-if="r.ingestion_status" class="repo-status-badge" :class="`repo-status-badge--${r.ingestion_status}`">{{ r.ingestion_status }}</span>
            <span v-if="r.versions.length" class="repo-list-item__versions">{{ r.versions.length }} version{{ r.versions.length !== 1 ? 's' : '' }}</span>
          </span>
          <span v-if="r.ingestion_status === 'running'" class="repo-list-item__progress">
            <LoadingSpinner />
          </span>
          <span v-if="r.ingestion_error" class="repo-list-item__error">{{ r.ingestion_error }}</span>
        </button>
      </div>

      <div class="repositories-detail">
        <div v-if="!selectedRepo" class="repositories-detail-empty">
          <p>Select a repository to view its statistics.</p>
        </div>
        <div v-else-if="selectedRepo && isIngesting" class="repo-progress-panel">
          <div class="repo-progress-header">
            <h2>{{ selectedRepo }}</h2>
            <span class="repo-status-badge repo-status-badge--running">ingesting</span>
          </div>
          <div class="repo-progress-body">
            <div class="repo-progress-spinner">
              <LoadingSpinner text="Ingesting repository" />
            </div>
            <div class="repo-progress-lines">
              <div
                v-for="(line, i) in progressLines"
                :key="i"
                class="repo-progress-line"
                :class="{ 'repo-progress-line--done': i < progressLines.length - 1 }"
              >
                <span class="repo-progress-line__icon">
                  <svg v-if="i < progressLines.length - 1" viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 8.5 6.5 12 13 4.5"/></svg>
                  <svg v-else class="repo-progress-line__spinner" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><path d="M.81 7.36a1.92 1.92 0 1 1 3.799.572A1.92 1.92 0 0 1 .81 7.36M8.826 3.033a1.92 1.92 0 1 1 3.755.806 1.92 1.92 0 0 1-3.755-.806M7.04 12.585a4.68 4.68 0 0 1-3.19-2.432 2.76 2.76 0 0 1-1.64.202 6.25 6.25 0 0 0 4.498 3.77c.45.098.908.144 1.364.141a2.74 2.74 0 0 1-.562-1.605 5 5 0 0 1-.47-.076M8.394 12.193a1.92 1.92 0 0 1 3.754.805 1.92 1.92 0 1 1-3.754-.805M12.943 11.89a6.3 6.3 0 0 0 1.22-2.587 6.3 6.3 0 0 0-.905-4.782 2.77 2.77 0 0 1-1.08 1.265 4.7 4.7 0 0 1-.154 4.674c.45.37.77.87.919 1.43M2.56 4.892a2.75 2.75 0 0 1 1.603.41 4.68 4.68 0 0 1 3.77-2.015q.012-.218.057-.433c.088-.411.268-.795.525-1.124A6.31 6.31 0 0 0 2.56 4.892"/></svg>
                </span>
                <span class="repo-progress-line__text">{{ line }}</span>
              </div>
            </div>
          </div>
        </div>
        <div v-else-if="statsLoading" class="repositories-detail-loading">
          <LoadingSpinner text="Loading statistics…" />
        </div>
        <div v-else-if="!stats" class="repositories-detail-empty">
          <p>No statistics available for this repository.</p>
        </div>
        <div v-else class="repo-stats">
          <div class="repo-stats__header">
            <div class="repo-stats__title">
              <h2>{{ stats.repository }}</h2>
              <a v-if="stats.url" :href="stats.url" target="_blank" rel="noopener" class="repo-stats__url">{{ stats.url }}</a>
            </div>
            <div class="repo-stats__toolbar">
              <div class="repo-version-dropdown">
                <select id="repo-version-select" v-model="selectedVersion" @change="loadStats">
                  <option v-for="v in stats.versions" :key="v" :value="v">{{ v }}</option>
                </select>
              </div>
              <button
                class="repo-toolbar-btn"
                type="button"
                aria-label="Resynchronize version"
                title="Resynchronize this version"
                :disabled="resyncing"
                @click="confirmResync"
              >
                <svg v-if="!resyncing" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>
                <svg v-else class="repo-toolbar-btn__spinning" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>
              </button>
              <button
                class="repo-toolbar-btn"
                type="button"
                aria-label="Ingest new versions"
                title="Ingest new versions"
                @click="openIngestVersionsModal"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
              </button>
              <button
                v-if="auth.isAdmin"
                class="repo-toolbar-btn repo-toolbar-btn--labeled"
                type="button"
                :disabled="stats.versions.length <= 1"
                @click="openDeleteVersionModal"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/></svg>
                <span>Delete version</span>
              </button>
              <button
                v-if="auth.isAdmin"
                class="repo-toolbar-btn repo-toolbar-btn--labeled repo-toolbar-btn--danger"
                type="button"
                @click="openDeleteRepoModal"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/><line x1="9" y1="13" x2="15" y2="13"/></svg>
                <span>Delete repository</span>
              </button>
            </div>
          </div>

          <div class="repo-stats__summary">
            <div class="repo-stat-card">
              <span class="repo-stat-card__value">{{ stats.totals.files }}</span>
              <span class="repo-stat-card__label">Files</span>
            </div>
            <div class="repo-stat-card">
              <span class="repo-stat-card__value">{{ stats.totals.symbols }}</span>
              <span class="repo-stat-card__label">Symbols</span>
            </div>
            <div class="repo-stat-card" v-for="rel in stats.relations" :key="rel.relation">
              <span class="repo-stat-card__value">{{ rel.count }}</span>
              <span class="repo-stat-card__label">{{ rel.relation }}</span>
            </div>
          </div>

          <section v-if="stats.languages.length" class="repo-stats__section">
            <h3>Language breakdown</h3>
            <div class="repo-lang-pie-wrap">
              <svg class="repo-lang-pie" viewBox="-1.1 -1.1 2.2 2.2" width="180" height="180">
                <circle cx="0" cy="0" r="1" fill="var(--bg-surface-alt)" />
                <template v-for="(seg, i) in pieSegments" :key="i">
                  <path :d="seg.path" :fill="seg.color" stroke="var(--bg-surface)" stroke-width="0.01" />
                </template>
                <circle v-if="pieSegments.length" cx="0" cy="0" r="0.5" fill="var(--bg-page)" />
              </svg>
              <div class="repo-lang-legend">
                <div v-for="lang in stats.languages" :key="lang.language" class="repo-lang-legend__item">
                  <span class="repo-lang-legend__swatch" :style="{ background: langColor(lang.language) }"></span>
                  <span class="repo-lang-legend__name">{{ lang.language }}</span>
                  <span class="repo-lang-legend__count">{{ lang.files }} ({{ lang.percentage.toFixed(1) }}%)</span>
                </div>
              </div>
            </div>
          </section>

          <section v-if="stats.symbols.length" class="repo-stats__section">
            <h3>Symbol types</h3>
            <div class="repo-symbol-grid">
              <div v-for="sym in stats.symbols" :key="sym.kind" class="repo-symbol-chip">
                <span class="repo-symbol-chip__count">{{ sym.count }}</span>
                <span class="repo-symbol-chip__label">{{ sym.kind }}</span>
                <span class="repo-symbol-chip__type">{{ sym.type }}</span>
              </div>
            </div>
          </section>
        </div>
      </div>
    </div>

    <div v-if="addModalOpen" class="modal" @click.self="closeAddModal">
      <div class="modal-content modal-content--wide">
        <button class="modal-close" type="button" @click="closeAddModal">✕</button>
        <h3>Add repository</h3>
        <div class="form-group">
          <label for="add-repo-url">Git URL</label>
          <input id="add-repo-url" v-model="addForm.url" type="text" placeholder="https://github.com/owner/repo.git" @keydown.enter="verifyUrl" :disabled="verifying" />
          <span v-if="verifyError" class="auth-error">{{ verifyError }}</span>
        </div>
        <div v-if="verifying" class="repo-verify-loading">
          <LoadingSpinner text="Connecting to repository…" />
        </div>
        <button
          v-if="!availableRefs.length && !verifying"
          class="p-button--positive is-dense"
          type="button"
          :disabled="!addForm.url.trim()"
          @click="verifyUrl"
        >Next</button>

        <template v-if="availableRefs.length">
          <div class="form-group">
            <label for="add-repo-name">Name</label>
            <input id="add-repo-name" v-model="addForm.name" type="text" placeholder="my-repo" />
          </div>
          <div class="form-group">
            <label>Refs <span class="form-field-hint-inline">(required — select at least one)</span></label>
            <div class="refs-multiselect">
              <div class="refs-multiselect__search">
                <input
                  v-model="refSearch"
                  type="search"
                  placeholder="Search refs…"
                  autocomplete="off"
                />
              </div>
              <div class="refs-multiselect__list">
                <label
                  v-for="r in filteredRefs"
                  :key="r.name"
                  class="refs-multiselect__item"
                  :class="{ 'refs-multiselect__item--selected': selectedRefs.has(r.name) }"
                >
                  <input
                    type="checkbox"
                    :value="r.name"
                    :checked="selectedRefs.has(r.name)"
                    @change="toggleRef(r.name)"
                  />
                  <span class="refs-multiselect__item-name">{{ r.name }}</span>
                  <span class="refs-multiselect__item-sha">{{ r.commit_sha.substring(0, 8) }}</span>
                </label>
                <p v-if="!filteredRefs.length" class="refs-multiselect__empty">No refs match "{{ refSearch }}"</p>
              </div>
              <div class="refs-multiselect__footer">
                <span class="refs-multiselect__count">{{ selectedRefs.size }} selected</span>
                <button type="button" class="p-button--base is-dense" @click="clearRefs">Clear</button>
              </div>
            </div>
          </div>
          <p v-if="addError" class="auth-error">{{ addError }}</p>
          <div class="modal-actions">
            <button class="p-button--base is-dense" type="button" @click="closeAddModal">Cancel</button>
            <button
              class="p-button--positive is-dense"
              type="button"
              :disabled="!addForm.name.trim() || !addForm.url.trim() || selectedRefs.size === 0 || adding"
              @click="submitAdd"
            >{{ adding ? 'Ingesting…' : 'Ingest' }}</button>
          </div>
        </template>
      </div>
    </div>

    <div v-if="ingestVersionsModalOpen" class="modal" @click.self="closeIngestVersionsModal">
      <div class="modal-content modal-content--wide">
        <button class="modal-close" type="button" @click="closeIngestVersionsModal">✕</button>
        <h3>Ingest new versions — {{ selectedRepo }}</h3>
        <p class="modal-lede">Harvest will connect to the repository and list all available refs. Select the ones you want to ingest.</p>
        <div v-if="ingestVerifying" class="repo-verify-loading">
          <LoadingSpinner text="Connecting to repository…" />
        </div>
        <button
          v-if="!ingestAvailableRefs.length && !ingestVerifying"
          class="p-button--base is-dense"
          type="button"
          @click="verifyForIngestVersions"
        >Connect & list refs</button>
        <span v-if="ingestVerifyError" class="auth-error">{{ ingestVerifyError }}</span>

        <template v-if="ingestAvailableRefs.length">
          <div class="form-group">
            <label>Refs <span class="form-field-hint-inline">(required — select at least one)</span></label>
            <div class="refs-multiselect">
              <div class="refs-multiselect__search">
                <input
                  v-model="ingestRefSearch"
                  type="search"
                  placeholder="Search refs…"
                  autocomplete="off"
                />
              </div>
              <div class="refs-multiselect__list">
                <label
                  v-for="r in filteredIngestRefs"
                  :key="r.name"
                  class="refs-multiselect__item"
                  :class="{
                    'refs-multiselect__item--selected': ingestSelectedRefs.has(r.name),
                    'refs-multiselect__item--disabled': ingestExistingVersions.has(r.name),
                  }"
                >
                  <input
                    type="checkbox"
                    :value="r.name"
                    :checked="ingestSelectedRefs.has(r.name)"
                    :disabled="ingestExistingVersions.has(r.name)"
                    @change="toggleIngestRef(r.name)"
                  />
                  <span class="refs-multiselect__item-name">{{ r.name }}</span>
                  <span v-if="ingestExistingVersions.has(r.name)" class="refs-multiselect__item-badge">already ingested</span>
                  <span v-else class="refs-multiselect__item-sha">{{ r.commit_sha.substring(0, 8) }}</span>
                </label>
                <p v-if="!filteredIngestRefs.length" class="refs-multiselect__empty">No refs match "{{ ingestRefSearch }}"</p>
              </div>
              <div class="refs-multiselect__footer">
                <span class="refs-multiselect__count">{{ ingestSelectedRefs.size }} selected</span>
                <button type="button" class="p-button--base is-dense" @click="clearIngestRefs">Clear</button>
              </div>
            </div>
          </div>
          <p v-if="ingestVersionsError" class="auth-error">{{ ingestVersionsError }}</p>
          <div class="modal-actions">
            <button class="p-button--base is-dense" type="button" @click="closeIngestVersionsModal">Cancel</button>
            <button
              class="p-button--positive is-dense"
              type="button"
              :disabled="ingestSelectedRefs.size === 0 || ingestVersionsSubmitting"
              @click="submitIngestVersions"
            >{{ ingestVersionsSubmitting ? 'Ingesting…' : 'Ingest' }}</button>
          </div>
        </template>
      </div>
    </div>

    <div v-if="deleteRepoModalOpen" class="modal" @click.self="closeDeleteRepoModal">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="closeDeleteRepoModal">✕</button>
        <h3>Delete repository</h3>
        <p class="modal-lede">Are you sure you want to delete <strong>{{ selectedRepo }}</strong> and all its versions, files, and symbols? This action cannot be undone.</p>
        <p v-if="deleteError" class="auth-error">{{ deleteError }}</p>
        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" @click="closeDeleteRepoModal">Cancel</button>
          <button
            class="p-button--negative is-dense"
            type="button"
            :disabled="deletingRepo"
            @click="confirmDeleteRepo"
          >{{ deletingRepo ? 'Deleting…' : 'Delete' }}</button>
        </div>
      </div>
    </div>

    <div v-if="deleteVersionModalOpen" class="modal" @click.self="closeDeleteVersionModal">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="closeDeleteVersionModal">✕</button>
        <h3>Delete version</h3>
        <p class="modal-lede">Are you sure you want to delete version <strong>{{ selectedVersion }}</strong> from <strong>{{ selectedRepo }}</strong>? This action cannot be undone.</p>
        <p v-if="deleteVersionError" class="auth-error">{{ deleteVersionError }}</p>
        <div class="modal-actions">
          <button class="p-button--base is-dense" type="button" @click="closeDeleteVersionModal">Cancel</button>
          <button
            class="p-button--negative is-dense"
            type="button"
            :disabled="deletingVersion"
            @click="confirmDeleteVersion"
          >{{ deletingVersion ? 'Deleting…' : 'Delete' }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import {
  fetchRepositories, addRepository, verifyRepository, fetchRepositoryStats,
  streamRepositoryProgress, deleteRepository, deleteVersion, ingestVersions,
  resyncRepository,
} from '../lib/api.js';
import { useAuthStore } from '../stores/auth.js';
import LoadingSpinner from '../components/deployment/LoadingSpinner.vue';

const auth = useAuthStore();

const LANG_COLORS = {
  rust: '#dea584',
  python: '#3776ab',
  go: '#00add8',
  javascript: '#f7df1e',
  typescript: '#3178c6',
  c: '#a8b9cc',
  cpp: '#00599c',
  'c++': '#00599c',
};
const FALLBACK_COLORS = ['#e95420', '#772953', '#5e5e5e', '#06c', '#335c81', '#5b3eb0', '#2d8a3e', '#c7162b'];

function langColor(lang) {
  if (LANG_COLORS[lang.toLowerCase()]) return LANG_COLORS[lang.toLowerCase()];
  const hash = [...lang].reduce((a, c) => a + c.charCodeAt(0), 0);
  return FALLBACK_COLORS[hash % FALLBACK_COLORS.length];
}

const repos = ref([]);
const listLoading = ref(true);
const selectedRepo = ref('');
const selectedVersion = ref('');
const stats = ref(null);
const statsLoading = ref(false);
const repoSearch = ref('');

const progressLines = ref([]);
let progressEs = null;

const addModalOpen = ref(false);
const addForm = ref({ name: '', url: '' });
const addError = ref('');
const adding = ref(false);
const verifying = ref(false);
const verifyError = ref('');
const availableRefs = ref([]);
const selectedRefs = ref(new Set());
const refSearch = ref('');

const ingestVersionsModalOpen = ref(false);
const ingestVerifying = ref(false);
const ingestVerifyError = ref('');
const ingestAvailableRefs = ref([]);
const ingestSelectedRefs = ref(new Set());
const ingestRefSearch = ref('');
const ingestExistingVersions = ref(new Set());
const ingestVersionsError = ref('');
const ingestVersionsSubmitting = ref(false);

const deleteRepoModalOpen = ref(false);
const deletingRepo = ref(false);
const deleteError = ref('');

const deleteVersionModalOpen = ref(false);
const deletingVersion = ref(false);
const deleteVersionError = ref('');

const resyncing = ref(false);

let pollTimer = null;

const filteredRepos = computed(() => {
  const q = repoSearch.value.trim().toLowerCase();
  if (!q) return repos.value;
  return repos.value.filter(r => r.name.toLowerCase().includes(q));
});

const filteredRefs = computed(() => {
  const q = refSearch.value.trim().toLowerCase();
  if (!q) return availableRefs.value;
  return availableRefs.value.filter(r => r.name.toLowerCase().includes(q));
});

const filteredIngestRefs = computed(() => {
  const q = ingestRefSearch.value.trim().toLowerCase();
  if (!q) return ingestAvailableRefs.value;
  return ingestAvailableRefs.value.filter(r => r.name.toLowerCase().includes(q));
});

const selectedRepoInfo = computed(() => repos.value.find(r => r.name === selectedRepo.value));
const isIngesting = computed(() => selectedRepoInfo.value?.ingestion_status === 'running' || selectedRepoInfo.value?.ingestion_status === 'pending');

const pieSegments = computed(() => {
  if (!stats.value || !stats.value.languages.length) return [];
  const langs = stats.value.languages;
  const total = langs.reduce((s, l) => s + l.files, 0);
  if (total === 0) return [];

  let cumAngle = -Math.PI / 2;
  return langs.map(lang => {
    const fraction = lang.files / total;
    const angle = fraction * 2 * Math.PI;
    const startAngle = cumAngle;
    const endAngle = cumAngle + angle;
    cumAngle = endAngle;

    const x1 = Math.cos(startAngle);
    const y1 = Math.sin(startAngle);
    const x2 = Math.cos(endAngle);
    const y2 = Math.sin(endAngle);
    const largeArc = angle > Math.PI ? 1 : 0;

    const path = `M 0 0 L ${x1} ${y1} A 1 1 0 ${largeArc} 1 ${x2} ${y2} Z`;

    return { path, color: langColor(lang.language), name: lang.language };
  });
});

async function loadRepos() {
  try {
    repos.value = await fetchRepositories();
  } catch {
    repos.value = [];
  } finally {
    listLoading.value = false;
  }
}

function selectRepo(name) {
  selectedRepo.value = name;
  selectedVersion.value = '';
  stats.value = null;
  progressLines.value = [];
  if (progressEs) { progressEs.close(); progressEs = null; }

  const repo = repos.value.find(r => r.name === name);
  if (repo && (repo.ingestion_status === 'running' || repo.ingestion_status === 'pending')) {
    connectProgress(name);
  } else {
    loadStats();
  }
}

function connectProgress(name) {
  if (progressEs) { progressEs.close(); progressEs = null; }
  progressLines.value = [];
  progressEs = streamRepositoryProgress(name, (event) => {
    if (event.type === 'progress') {
      progressLines.value = [...progressLines.value, event.message];
    } else if (event.type === 'completed') {
      progressLines.value = [...progressLines.value, 'Ingestion completed successfully.'];
      if (progressEs) { progressEs.close(); progressEs = null; }
      loadRepos().then(() => loadStats());
    } else if (event.type === 'failed') {
      progressLines.value = [...progressLines.value, `Error: ${event.error}`];
      if (progressEs) { progressEs.close(); progressEs = null; }
      loadRepos();
    } else if (event.type === 'done') {
      if (progressEs) { progressEs.close(); progressEs = null; }
      loadRepos().then(() => loadStats());
    }
  });
}

async function loadStats() {
  if (!selectedRepo.value || isIngesting.value) return;
  statsLoading.value = true;
  try {
    const result = await fetchRepositoryStats(selectedRepo.value, selectedVersion.value || null);
    stats.value = result;
    if (result && !selectedVersion.value && result.versions.length) {
      selectedVersion.value = result.version;
    }
  } catch {
    stats.value = null;
  } finally {
    statsLoading.value = false;
  }
}

function openAddModal() {
  addForm.value = { name: '', url: '' };
  addError.value = '';
  verifyError.value = '';
  availableRefs.value = [];
  selectedRefs.value = new Set();
  refSearch.value = '';
  addModalOpen.value = true;
}

function closeAddModal() {
  addModalOpen.value = false;
}

async function verifyUrl() {
  const url = addForm.value.url.trim();
  if (!url) return;

  verifying.value = true;
  verifyError.value = '';
  availableRefs.value = [];
  selectedRefs.value = new Set();

  try {
    const result = await verifyRepository(url);
    availableRefs.value = result.refs;

    if (!addForm.value.name.trim()) {
      const parts = url.replace(/\.git$/, '').split('/');
      const inferred = parts[parts.length - 1] || '';
      addForm.value.name = inferred;
    }
  } catch (e) {
    verifyError.value = e.message || 'Failed to connect to repository';
  } finally {
    verifying.value = false;
  }
}

function toggleRef(name) {
  const next = new Set(selectedRefs.value);
  if (next.has(name)) next.delete(name);
  else next.add(name);
  selectedRefs.value = next;
}

function clearRefs() {
  selectedRefs.value = new Set();
}

async function submitAdd() {
  const name = addForm.value.name.trim();
  const url = addForm.value.url.trim();
  const refs = [...selectedRefs.value];

  if (!name || !url || refs.length === 0) return;

  adding.value = true;
  addError.value = '';
  try {
    await addRepository({ name, url, refs });
    addModalOpen.value = false;
    await loadRepos();
    selectRepo(name);
    startPolling();
  } catch (e) {
    addError.value = e.message || 'Failed to start ingestion';
  } finally {
    adding.value = false;
  }
}

function openIngestVersionsModal() {
  ingestAvailableRefs.value = [];
  ingestSelectedRefs.value = new Set();
  ingestRefSearch.value = '';
  ingestVerifyError.value = '';
  ingestVersionsError.value = '';
  ingestExistingVersions.value = new Set(stats.value?.versions ?? []);
  ingestVersionsModalOpen.value = true;
}

function closeIngestVersionsModal() {
  ingestVersionsModalOpen.value = false;
}

async function verifyForIngestVersions() {
  if (!stats.value?.url) {
    ingestVerifyError.value = 'Repository URL not found';
    return;
  }

  ingestVerifying.value = true;
  ingestVerifyError.value = '';
  ingestAvailableRefs.value = [];
  ingestSelectedRefs.value = new Set();

  try {
    const result = await verifyRepository(stats.value.url);
    ingestAvailableRefs.value = result.refs;
  } catch (e) {
    ingestVerifyError.value = e.message || 'Failed to connect to repository';
  } finally {
    ingestVerifying.value = false;
  }
}

function toggleIngestRef(name) {
  if (ingestExistingVersions.value.has(name)) return;
  const next = new Set(ingestSelectedRefs.value);
  if (next.has(name)) next.delete(name);
  else next.add(name);
  ingestSelectedRefs.value = next;
}

function clearIngestRefs() {
  ingestSelectedRefs.value = new Set();
}

async function submitIngestVersions() {
  const refs = [...ingestSelectedRefs.value];
  if (refs.length === 0) return;

  ingestVersionsSubmitting.value = true;
  ingestVersionsError.value = '';
  try {
    await ingestVersions(selectedRepo.value, refs);
    ingestVersionsModalOpen.value = false;
    await loadRepos();
    selectRepo(selectedRepo.value);
    startPolling();
  } catch (e) {
    ingestVersionsError.value = e.message || 'Failed to start ingestion';
  } finally {
    ingestVersionsSubmitting.value = false;
  }
}

function openDeleteRepoModal() {
  deleteError.value = '';
  deleteRepoModalOpen.value = true;
}

function closeDeleteRepoModal() {
  deleteRepoModalOpen.value = false;
}

async function confirmDeleteRepo() {
  deletingRepo.value = true;
  deleteError.value = '';
  try {
    await deleteRepository(selectedRepo.value);
    deleteRepoModalOpen.value = false;
    selectedRepo.value = '';
    selectedVersion.value = '';
    stats.value = null;
    await loadRepos();
  } catch (e) {
    deleteError.value = e.message || 'Failed to delete repository';
  } finally {
    deletingRepo.value = false;
  }
}

function openDeleteVersionModal() {
  deleteVersionError.value = '';
  deleteVersionModalOpen.value = true;
}

function closeDeleteVersionModal() {
  deleteVersionModalOpen.value = false;
}

async function confirmDeleteVersion() {
  deletingVersion.value = true;
  deleteVersionError.value = '';
  try {
    await deleteVersion(selectedRepo.value, selectedVersion.value);
    deleteVersionModalOpen.value = false;
    selectedVersion.value = '';
    await loadRepos();
    await loadStats();
  } catch (e) {
    deleteVersionError.value = e.message || 'Failed to delete version';
  } finally {
    deletingVersion.value = false;
  }
}

async function confirmResync() {
  if (!selectedRepo.value || !selectedVersion.value) return;
  resyncing.value = true;
  try {
    await resyncRepository(selectedRepo.value, selectedVersion.value);
    await loadRepos();
    selectRepo(selectedRepo.value);
    startPolling();
  } catch (e) {
    resyncing.value = false;
  }
}

function startPolling() {
  if (pollTimer) clearInterval(pollTimer);
  pollTimer = setInterval(async () => {
    await loadRepos();
    const hasRunning = repos.value.some(r => r.ingestion_status === 'running' || r.ingestion_status === 'pending');
    if (!hasRunning) {
      clearInterval(pollTimer);
      pollTimer = null;
      resyncing.value = false;
    }
  }, 3000);
}

onMounted(async () => {
  await loadRepos();
  const hasRunning = repos.value.some(r => r.ingestion_status === 'running' || r.ingestion_status === 'pending');
  if (hasRunning) startPolling();
});

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
  if (progressEs) progressEs.close();
});
</script>

<template>
  <div class="repo-browser">
    <header class="repo-browser__header">
      <div class="repo-browser__title">
        <router-link to="/code/repositories" class="code-back">Code repositories</router-link>
        <h1>{{ name }}</h1>
        <a v-if="url" :href="url" class="repo-browser__url" target="_blank" rel="noopener noreferrer">{{ url }}</a>
      </div>
      <label v-if="versions.length" class="repo-browser__version">
        <span>Version</span>
        <select :value="version" @change="go({ version: $event.target.value, path: ROOT, isFile: false })">
          <option v-for="option in versions" :key="option.name" :value="option.name">{{ option.name }}</option>
        </select>
      </label>
    </header>

    <nav class="p-breadcrumbs repo-browser__breadcrumbs" aria-label="Path">
      <ol class="p-breadcrumbs__items">
        <li class="p-breadcrumbs__item">
          <router-link :to="linkTo({ path: ROOT, isFile: false })">{{ name }}</router-link>
        </li>
        <li v-for="(crumb, index) in crumbs" :key="crumb.path" class="p-breadcrumbs__item">
          <span v-if="index === crumbs.length - 1" aria-current="page">{{ crumb.name }}</span>
          <router-link v-else :to="linkTo({ path: crumb.path, isFile: false })">{{ crumb.name }}</router-link>
        </li>
      </ol>
    </nav>

    <p v-if="error" class="code-error repo-browser__error" role="alert">{{ error }}</p>

    <div v-if="version" class="repo-browser__body">
      <aside class="repo-browser__tree">
        <FileTree :key="version" label="Files" :load-children="children" :selected-path="location.path" @select="openEntry" />
      </aside>
      <div class="repo-browser__content">
        <p v-if="loading" class="repo-browser__loading u-text--muted"><i class="p-icon--spinner u-animation--spin" /> Loading…</p>
        <FileView v-else-if="file" :file="file" :raw-url="rawUrl" :line="location.line" @select-line="selectLine" />
        <DirectoryView v-else-if="entries" :entries="entries" :readme="readme" :link-to="(entry) => linkTo(entryLocation(entry))" />
      </div>
    </div>
    <p v-else-if="loaded && !error" class="repo-browser__empty u-text--muted">This repository has no ingested version yet.</p>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import FileTree from '../components/code/browser/FileTree.vue';
import FileView from '../components/code/browser/FileView.vue';
import DirectoryView from '../components/code/browser/DirectoryView.vue';
import { getRepositoryFile, listRepositoryTree, listRepositoryVersions, repositoryRawUrl } from '../lib/code-api.js';
import {
  ROOT,
  breadcrumbs,
  cachedLoader,
  findReadme,
  isDirectory,
  locationFromQuery,
  queryFromLocation,
} from '../lib/repository-browser.js';

const props = defineProps({
  name: { type: String, required: true },
});

const route = useRoute();
const router = useRouter();

const url = ref('');
const versions = ref([]);
const defaultVersion = ref(null);
const loaded = ref(false);
const entries = ref(null);
const readme = ref(null);
const file = ref(null);
const loading = ref(false);
const error = ref('');

const location = computed(() => locationFromQuery(route.query));
const version = computed(() => location.value.version ?? defaultVersion.value);
const crumbs = computed(() => breadcrumbs(location.value.path));
const children = computed(() => cachedLoader((path) => listRepositoryTree(props.name, version.value, path)));
const rawUrl = computed(() => repositoryRawUrl(props.name, version.value, file.value?.path ?? ''));

const entryLocation = (entry) => ({ path: entry.path, isFile: !isDirectory(entry) });

function linkTo(target) {
  return { query: queryFromLocation({ version: location.value.version, line: null, ...target }) };
}

function go(target, { replace = false } = {}) {
  const destination = linkTo(target);
  return replace ? router.replace(destination) : router.push(destination);
}

const openEntry = (entry) => go(entryLocation(entry));
const selectLine = (line) => go({ path: location.value.path, isFile: true, line }, { replace: true });

async function loadVersions() {
  loaded.value = false;
  error.value = '';
  try {
    const listing = await listRepositoryVersions(props.name);
    url.value = listing.url;
    versions.value = listing.versions;
    defaultVersion.value = listing.default;
  } catch (e) {
    error.value = e.message;
  } finally {
    loaded.value = true;
  }
}

async function readmeOf(listing) {
  const entry = findReadme(listing);
  if (!entry) return null;
  const readmeFile = await getRepositoryFile(props.name, version.value, entry.path).catch(() => null);
  return readmeFile?.kind === 'text' ? readmeFile : null;
}

async function viewOf({ path, isFile }) {
  if (isFile) {
    return { file: await getRepositoryFile(props.name, version.value, path), entries: null, readme: null };
  }
  const listing = await children.value(path);
  return { file: null, entries: listing, readme: await readmeOf(listing) };
}

let latestRequest = 0;

async function showLocation() {
  if (!version.value) return;
  const request = ++latestRequest;
  const isLatest = () => request === latestRequest;
  loading.value = true;
  error.value = '';
  try {
    const view = await viewOf(location.value);
    if (!isLatest()) return;
    file.value = view.file;
    entries.value = view.entries;
    readme.value = view.readme;
  } catch (e) {
    if (isLatest()) error.value = e.message;
  } finally {
    if (isLatest()) loading.value = false;
  }
}

watch(() => props.name, loadVersions, { immediate: true });
watch([version, () => location.value.path, () => location.value.isFile], showLocation, { immediate: true });
</script>
